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

if not __debug__:
    raise SystemExit('unsupported optimized Python execution: qualification requires assertions; remove -O and PYTHONOPTIMIZE')

ROOT = Path(__file__).resolve().parents[1]
base = ''

DOCKER_TIMEOUT = 120
INVENTORY_RESOURCES = ('container', 'network', 'volume')
PNG = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aD1sAAAAASUVORK5CYII=')


def run_docker(args, timeout, input_data=None):
    """Own CLI/plugin descendants and never expose private timeout argv or output."""
    process = subprocess.Popen(['docker', *args], cwd=ROOT, start_new_session=True,
                               stdin=subprocess.PIPE if input_data is not None else None,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE)
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
            error.add_note('Docker local process cleanup exceeded 10 s')
        finally:
            for stream in (process.stdin, process.stdout, process.stderr):
                if stream is not None:
                    stream.close()
        if isinstance(error, subprocess.TimeoutExpired):
            raise subprocess.TimeoutExpired(['docker', args[0]], timeout) from None
        raise
    result = subprocess.CompletedProcess(['docker', args[0]], process.returncode, stdout, stderr)
    result.check_returncode()
    return result


def docker(*args, input=None, timeout=DOCKER_TIMEOUT):
    """Run Docker under a finite deadline; only ordinary failures expose stderr."""
    if not 0 < timeout < math.inf:
        raise ValueError('Docker operation timeout must be finite and positive')
    try:
        return run_docker(args, timeout, input)
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


class HTTPStatusError(AssertionError):
    """A response whose status differs from the caller's expectation, with its headers and body."""

    def __init__(self, path, status, expected, headers, body):
        super().__init__((path, status, expected))
        self.path, self.status, self.expected, self.headers, self.body = path, status, expected, headers, body


def request(path, data=None, token=None, headers=None, expected=200, timeout=15, method=None):
    """Issue an HTTP request, require its status and return the body, error bodies included."""
    fields = dict(headers or {})
    if token:
        fields['Authorization'] = 'Bearer ' + token
    if data is not None:
        fields['Content-Type'] = 'application/json'
    req = urllib.request.Request(base + path, data=None if data is None else json.dumps(data).encode(),
                                 headers=fields, method=method)
    try:
        response = urllib.request.urlopen(req, timeout=timeout)
    except urllib.error.HTTPError as error:
        response = error
    with closing(response):
        body = response.read()
        if response.status != expected:
            raise HTTPStatusError(path, response.status, expected, response.headers, body)
        return body


def wait_until(probe, failure, timeout, interval=0.1, transient=()):
    """Return the first truthy probe() result; retry `transient` errors until the deadline."""
    deadline = time.monotonic() + timeout
    last_error = None
    while True:
        try:
            result = probe()
        except transient as error:
            last_error = error
        else:
            if result:
                return result
            last_error = None
        if time.monotonic() >= deadline:
            raise AssertionError(failure if last_error is None else f'{failure}: {last_error!r}')
        time.sleep(interval)


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
    print('PASS: WebSocket upgrade without the creator account is refused (403)')
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
        print(f'PASS: {phase} {path} ({elapsed:.3f}s)')


def ready():
    """Wait for the disposable HTTP endpoint to accept requests."""
    wait_until(lambda: request('/api/health', timeout=2), 'QA server did not become ready', 20, 0.5,
               transient=(OSError, AssertionError))


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


def scan_status():
    """Read the public scan status."""
    return json.loads(request('/api/scan/status'))


def delete_media(media_id, token, expected):
    """DELETE one media row, retrying only the server's transient 409 refusal; print the retry count.

    After a scan, its conversion worker holds the global conversion slot while it checks whether a
    video needs converting, even when it will convert nothing. When that slot is the only conflict,
    deletion answers 409 with Retry-After and changes nothing, so a retry is safe. A 409 without
    Retry-After (a referenced item) and any other status must match at once.
    """
    retries = 0

    def attempt():
        nonlocal retries
        try:
            request(f'/api/media/{media_id}', token=token, method='DELETE', expected=expected)
        except HTTPStatusError as error:
            if error.status != 409 or error.headers.get('Retry-After') is None:
                raise
            retries += 1
            return False
        return True
    try:
        wait_until(attempt, f'media {media_id} delete stayed refused with transient 409', 10, 0.2)
    finally:
        print(f'delete media {media_id}: {retries} transient 409 retries')


def conflict_retry_after(media_id, token):
    """DELETE media whose deletion must be refused with 409; return the refusal's Retry-After, if any."""
    try:
        request(f'/api/media/{media_id}', token=token, method='DELETE', expected=204)
    except HTTPStatusError as error:
        if error.status != 409:
            raise
        return error.headers.get('Retry-After')
    raise AssertionError(f'media {media_id} was deleted while its deletion had to be refused')


def wait_for_scan_idle():
    """Retry only scan-status transport failures within the disposable scan budget."""
    wait_until(lambda: not json.loads(request('/api/scan/status', timeout=2))['scanning'],
               'disposable scan did not finish within 30 seconds', 30, 0.5, transient=(OSError,))


def settle():
    """Wait for the proxy to answer and the startup scan to finish."""
    ready()
    wait_for_scan_idle()


@contextmanager
def read_snapshot(database):
    """Open the fixture DB read-only inside one consistent transaction."""
    with closing(sqlite3.connect(database.resolve().as_uri() + '?mode=ro', uri=True)) as connection:
        connection.row_factory = sqlite3.Row
        connection.execute('BEGIN')
        yield connection


def progress_rows(connection, media_id):
    """Return the fixture device's progress rows for one media item."""
    return [dict(row) for row in connection.execute(
        'SELECT identity_id,media_id,position,completed,updated_at FROM progress WHERE identity_id=1 AND media_id=?',
        (media_id,))]


def legacy_fixture_snapshot(database):
    """Read only nonsecret media/progress columns from the disposable fixture DB."""
    with read_snapshot(database) as connection:
        assert connection.execute('PRAGMA quick_check').fetchone()[0] == 'ok'
        media = [dict(row) for row in connection.execute(
            'SELECT id,kind,path,title,size_bytes,mtime_secs,transcode_path FROM media WHERE id IN (1,2) ORDER BY id')]
        return {'media': media, 'progress': progress_rows(connection, 2)}


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
    with read_snapshot(database) as connection:
        media = dict(connection.execute(
            'SELECT id,kind,path,title,size_bytes,mtime_secs,meta_json FROM media WHERE id=3').fetchone())
        metadata = json.loads(media.pop('meta_json'))
        return {'media': media, 'metadata': metadata, 'progress': progress_rows(connection, 3)[0]}


def comic_cache_failure_snapshot(database):
    """Read the faulted synthetic comic row, progress and trigger without writing."""
    with read_snapshot(database) as connection:
        media = dict(connection.execute(
            'SELECT id,kind,path,title,size_bytes,mtime_secs,meta_json FROM media WHERE id=4').fetchone())
        trigger = connection.execute("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='qa_reject_comic_cache'").fetchone()[0]
        return {'media': media, 'progress': progress_rows(connection, 4)[0], 'trigger': trigger}


def inventory_command(project, resource):
    """List task-owned resources by label; stopped containers need -a to appear."""
    flags = ('-a',) if resource == 'container' else ()
    return (resource, 'ls', '-q', *flags, '--filter', f'label=com.docker.compose.project={project}')


def assert_project_removed(project):
    """Reject any remaining task-owned containers, networks or volumes."""
    for resource in INVENTORY_RESOURCES:
        remaining = docker(*inventory_command(project, resource)).stdout.strip()
        assert not remaining, (resource, project, remaining)


def qualified_thumbnail(data, media_id, saved):
    """Capture the exact DB-selected UUID JPEG inside the owned fixture thumbnail mount."""
    assert isinstance(saved, str), 'thumbnail publication missing'
    prefix = f'/data/thumbs/{media_id}-'
    assert saved.startswith(prefix) and saved.endswith('.jpg'), 'thumbnail outside exact fixture namespace'
    token = saved[len(prefix):-4]
    # A canonical UUID has no separators, so the name cannot leave the thumbnail directory.
    assert str(uuid.UUID(token)) == token, 'thumbnail UUID is not canonical'
    thumbs = data / 'thumbs'
    thumbnail = thumbs / f'{media_id}-{token}.jpg'
    assert not thumbs.is_symlink() and thumbs.is_dir(), 'thumbnail directory redirected'
    assert not thumbnail.is_symlink() and thumbnail.is_file(), 'thumbnail is not a regular file'
    content = thumbnail.read_bytes()
    assert content.startswith(b'\xff\xd8\xff') and content.endswith(b'\xff\xd9'), 'thumbnail is not JPEG'
    return thumbnail, content


def transcode_outputs(container):
    """List converted audio.mp4 files in the container's transcode cache."""
    found = docker('exec', container, 'find', '/data/transcodes', '-type', 'f', '-name', 'audio.mp4').stdout
    return found.decode().splitlines()


def synthetic_video(container, path, color, side, seconds, frequency):
    """Encode a short H264/AC3 Matroska clip inside the container and return its bytes."""
    docker('exec', container, 'ffmpeg', '-v', 'error', '-f', 'lavfi', '-i',
           f'color=c={color}:s={side}x{side}:r=10:d={seconds}', '-f', 'lavfi', '-i',
           f'sine=frequency={frequency}:sample_rate=48000:duration={seconds}', '-c:v', 'libx264',
           '-pix_fmt', 'yuv420p', '-c:a', 'ac3', '-shortest', path)
    return docker('exec', container, 'cat', path).stdout


def comic_archive():
    """Build a one-page CBZ whose page is the PNG fixture."""
    archive = io.BytesIO()
    with zipfile.ZipFile(archive, 'w') as comic:
        comic.writestr('001.png', PNG)
    return archive.getvalue()


def qa_config(args, project):
    """Derive a disposable project's Compose config from compose.yaml and the prebuilt images."""
    config = json.loads(docker('compose', '-f', str(ROOT / 'compose.yaml'), 'config', '--format', 'json').stdout)
    config['name'] = project
    for section in ('networks', 'volumes'):
        for key, resource in config[section].items():
            resource['name'] = f'{project}_{key}'
    for service in config['services'].values():
        service.pop('build', None)
    config['services']['app']['image'] = args.app_image
    config['services']['proxy']['image'] = args.proxy_image
    config['services']['proxy']['ports'][0]['published'] = '0'
    return config


def project_compose(project, config):
    """Return a runner that feeds the current in-memory config to Compose on stdin."""
    def compose(*commands):
        """Run each Compose command against this fixture project and its in-memory config."""
        return docker('compose', '-p', project, '-f', '-', *commands, input=json.dumps(config).encode())
    return compose


def start_project(compose):
    """Bring the project up, point the HTTP helpers at its loopback proxy port and wait for it."""
    global base
    compose('up', '-d', '--wait', '--wait-timeout', '90')
    address = compose('port', 'proxy', '8080').stdout.decode().strip()
    assert address.startswith('127.0.0.1:'), address
    base = 'http://' + address
    ready()


def container_id(compose, service):
    """Return the running container id of one Compose service."""
    return compose('ps', '-q', service).stdout.decode().strip()


def restart_app(compose):
    """Restart the app and wait for HTTP and the startup scan."""
    compose('restart', 'app')
    settle()


def upstream_contract_qualification(args):
    """Run fresh two-root/admin/queued-ingest HTTP paths on the caller-qualified image."""
    project = 'frayd-upstream-qa-' + secrets.token_hex(4)
    print(f'SCENARIO upstream contract: {project}')
    config = qa_config(args, project)
    # This fake-tool scenario qualifies enabled core ingestion; the ordinary Docker default stays disabled.
    config['services']['app']['environment']['FRAYD_DOWNLOADS_ENABLED'] = '1'
    config['services']['app']['environment']['FRAYD_DOWNLOAD_BACKEND'] = 'yt-dlp'
    for service in config['services'].values():
        service['restart'] = 'no'
    with tempfile.TemporaryDirectory(prefix=project) as directory:
        folder = Path(directory)
        data, roots, tools = (folder / key for key in ('data', 'roots', 'tools'))
        for path in (data, roots, tools, roots / 'a', roots / 'b',
                     data / 'thumbs', data / 'transcodes', roots / 'a/Internet'):
            path.mkdir()
            path.chmod(0o2770)
        # This owned synthetic DB is shared with the image's actual uid 10001.
        # Precreating it fixes both host SQL access and inherited WAL/SHM modes.
        (data / 'frayd.sqlite').touch(mode=0o660)
        (data / 'frayd.sqlite').chmod(0o660)
        source_bytes = comic_archive()
        for path in (roots / 'a/first.cbz', roots / 'b/second.cbz'):
            path.write_bytes(source_bytes)
        download_bytes = (ROOT / 'tests/fixtures/native-resume.mp4').read_bytes()
        (tools / 'download.mp4').write_bytes(download_bytes)
        (tools / 'yt-dlp').write_text('#!/bin/sh\nif [ "$1" = --version ]; then echo owned-fixture; exit 0; fi\n'
                                    'set -eu\noutput=\nwhile [ "$#" -gt 0 ]; do\n'
                                    'case "$1" in -o) output="$2"; shift 2;; *) shift;; esac\ndone\n'
                                    '[ -n "$output" ] || exit 2\n'
                                    'cp /qa/download.mp4 "${output%/*}/queued.mp4"\n')
        (tools / 'ffprobe').write_text('#!/bin/sh\ncase "$*" in */roots/b/block.mp4*) '
                                     'if [ -e /data/hold-probe ]; then touch /data/probe-entered; '
                                     'while [ -e /data/hold-probe ]; do sleep 0.05; done; fi;; esac\n'
                                     'case "$*" in *stream=codec_name*) if [ -e /data/hold-conversion ]; then '
                                     'touch /data/conversion-entered; while [ -e /data/hold-conversion ]; do sleep 0.05; done; '
                                     'fi;; esac\nexec /usr/bin/ffprobe "$@"\n')
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

        compose = project_compose(project, config)

        def sql(statement, parameters=()):
            """Execute and commit a fixture SQL statement; return all result rows."""
            with closing(sqlite3.connect(data / 'frayd.sqlite', timeout=10)) as db:
                result = db.execute(statement, parameters).fetchall()
                db.commit()
                return result

        def settings():
            """Read the effective admin settings using the owner capability."""
            return json.loads(request('/api/admin/settings', token=token))

        def ingest_status(*states):
            """Return the scan status once the ingest phase is one of states."""
            status = scan_status()
            return status if status['ingesting'] in states else None

        def library():
            """Read the visible library through the disposable proxy."""
            return json.loads(request('/api/library', token=token))

        def post_settings(payload, expected=204):
            """Post an admin settings change and assert the expected HTTP status."""
            return request('/api/admin/settings', payload, token, expected=expected, method='POST')

        with cleanup_project(compose):
            start_project(compose)
            assert json.loads(request('/api/health')) == {'status': 'ok'}
            binary_build = json.loads(compose('exec', '-T', 'app', 'frayd-server', '--build-info').stdout)
            with closing(urllib.request.urlopen(base + '/api/build', timeout=5)) as build_response:
                assert build_response.status == 200
                assert build_response.headers['Cache-Control'] == 'no-store'
                assert json.load(build_response) == binary_build
            print('PASS: build info CLI matches the uncached /api/build response')
            owner = json.loads(request('/api/setup', {'email': 'upstream-qa@example.invalid',
                                                      'password': secrets.token_urlsafe(32)}, method='POST'))
            token = owner['token']
            assert settings()['effective_library_paths'] == []
            assert scan_status() == {
                'scanning': False, 'scan_error': None, 'library_configured': False, 'library_count': 0, 'ingesting': None}
            request('/api/scan', token=token, method='POST', expected=501)
            post_settings({'library_paths': ['/roots/a', '/roots/b']})
            request('/api/scan', token=token, method='POST', expected=202)
            wait_for_scan_idle()
            assert scan_status() == {
                'scanning': False, 'scan_error': None, 'library_configured': True, 'library_count': 2, 'ingesting': None}
            items = library()
            ids = dict(sql('SELECT path,id FROM media'))
            assert set(ids) == {'/roots/a/first.cbz', '/roots/b/second.cbz'}, ids
            assert {item['id'] for item in items} == set(ids.values())
            media_id = ids['/roots/a/first.cbz']
            request('/api/progress', {'media_id': media_id, 'position': 0.5}, token, expected=204, method='POST')
            request('/api/annotations', {'media_id': media_id, 'kind': 'bookmark', 'spine_idx': 0,
                                         'cfi': 'qa-root-preservation'}, token, expected=201, method='POST')
            before_settings = settings()
            post_settings({'library_paths': ['/roots/a', '/missing-owned-qa'], 'site_name': 'must-not-commit'}, 400)
            assert settings() == before_settings
            sql("CREATE TRIGGER qa_settings_failure BEFORE INSERT ON settings WHEN NEW.key='qa_failure' BEGIN SELECT RAISE(ABORT,'owned QA'); END")
            post_settings({'library_paths': ['/roots/b'], 'qa_failure': True}, 500)
            assert settings() == before_settings
            sql('DROP TRIGGER qa_settings_failure')
            post_settings({'library_paths': ['/roots/b']})
            restart_app(compose)
            assert dict(sql('SELECT path,id FROM media')) == ids
            assert {item['id'] for item in library()} == set(ids.values())
            assert json.loads(request(f'/api/progress/{media_id}', token=token))['position'] == 0.5
            assert len(json.loads(request(f'/api/annotations/{media_id}', token=token))) == 1
            assert request(f'/api/stream/{media_id}') == source_bytes
            assert settings()['effective_library_paths'] == ['/roots/b']
            post_settings({'library_path': '/roots/a'})
            post_settings({'library_paths': []})
            restart_app(compose)
            assert settings()['effective_library_paths'] == []
            assert sql("SELECT key FROM settings WHERE key='library_path'") == []
            print('PASS: roots added, removed and restarted; missing path rejected (400); failed write rolled back (500)')

            app['environment']['FRAYD_LIBRARY'] = '/roots/b'
            compose('up', '-d', '--no-deps', 'app')
            settle()
            assert settings()['library_locked'] is True
            assert settings()['library_paths'] == []
            assert settings()['effective_library_paths'] == ['/roots/b']
            post_settings({'library_paths': ['/roots/a']}, 409)
            app['environment']['FRAYD_LIBRARY'] = ''
            compose('up', '-d', '--no-deps', 'app')
            settle()
            post_settings({'library_paths': ['/roots/a', '/roots/b']})

            # An actual scan blocks in our owned probe while the fake downloader exits.
            # Indexing must queue behind that scan, never advertise completion early.
            # The new video's conversion check then holds the conversion slot in its own probe.
            (data / 'hold-probe').touch()
            (data / 'hold-conversion').touch()
            (roots / 'b/block.mp4').write_bytes(b'owned invalid probe input')
            request('/api/scan', token=token, method='POST', expected=202)
            wait_until(lambda: (data / 'probe-entered').exists(), 'scan never reached owned blocking probe', 10, 0.05)
            request('/api/ingest', {'url': 'https://example.invalid/owned-queue'}, token, expected=202, method='POST')
            status = wait_until(lambda: ingest_status('indexing'), 'ingest never reported queued indexing', 10, 0.05)
            assert status['scanning'] is True
            request('/api/ingest', {'url': 'https://example.invalid/owned-queue'}, token, expected=409, method='POST')
            request('/api/scan', token=token, method='POST', expected=409)
            post_settings({'library_paths': []}, 409)
            (data / 'hold-probe').unlink()
            wait_until(lambda: (data / 'conversion-entered').exists(),
                       'conversion worker never reached owned blocking probe', 10, 0.05)
            idle_id = ids['/roots/b/second.cbz']
            assert conflict_retry_after(idle_id, token) == '1'
            assert sql('SELECT id FROM media WHERE id=?', (idle_id,)) == [(idle_id,)]
            assert conflict_retry_after(media_id, token) is None
            (data / 'hold-conversion').unlink()
            status = wait_until(lambda: ingest_status('done', 'failed'), 'ingest did not finish', 30)
            assert status['ingesting'] == 'done', status
            queued_id = sql("SELECT id FROM media WHERE path='/roots/a/Internet/queued.mp4'")[0][0]
            assert queued_id in {item['id'] for item in library()}
            wait_for_scan_idle()
            print('PASS: ingest queues behind a running scan and is indexed before done; busy requests get 409')
            print('PASS: held conversion slot refuses an unreferenced delete with Retry-After; '
                  'a referenced delete answers 409 without it')

            request('/api/media/999999', token=token, method='DELETE', expected=404)
            request(f'/api/media/{media_id}?identity=qa-nonadmin', method='DELETE', expected=403)
            assert conflict_retry_after(media_id, token) is None
            own_id = sql("SELECT id FROM identities WHERE key='upstream-qa@example.invalid'")[0][0]
            target = 10001
            sql("INSERT INTO identities(id,kind,key) VALUES(?,'account','qa-target'),(10002,'account','qa-unrelated')", (target,))
            sql('INSERT INTO user_perms(identity_id,is_admin) VALUES(?,0),(10002,0)', (target,))
            sql('INSERT INTO progress(identity_id,media_id,position) VALUES(?,?,2),(10002,?,3)', (target, media_id, media_id))
            sql("INSERT INTO annotations(identity_id,media_id,kind,spine_idx,cfi) VALUES(?,?,'bookmark',0,'target'),(10002,?,'bookmark',0,'other')", (target, media_id, media_id))
            sql('INSERT INTO sessions(token,identity_id,expires_at) VALUES(?,?,unixepoch()+1000)', (secrets.token_hex(32), target))
            request('/api/admin/perms', {'identity_id': target, 'allowed_kinds': ['episode']}, token, expected=204, method='POST')
            request('/api/admin/perms', {'identity_id': target, 'allowed_kinds': None}, token, expected=204, method='POST')
            assert sql('SELECT allowed_kinds FROM user_perms WHERE identity_id=?', (target,)) == [(None,)]
            request(f'/api/admin/users/{own_id}', token=token, method='DELETE', expected=409)
            sql("CREATE TRIGGER qa_user_failure BEFORE DELETE ON identities WHEN OLD.id=10001 BEGIN SELECT RAISE(ABORT,'owned QA'); END")
            before = [sql(f'SELECT COUNT(*) FROM {table} WHERE identity_id=?', (target,)) for table in ('sessions', 'user_perms', 'progress', 'annotations')]
            request(f'/api/admin/users/{target}', token=token, method='DELETE', expected=500)
            assert before == [sql(f'SELECT COUNT(*) FROM {table} WHERE identity_id=?', (target,)) for table in ('sessions', 'user_perms', 'progress', 'annotations')]
            sql('DROP TRIGGER qa_user_failure')
            request(f'/api/admin/users/{target}', token=token, method='DELETE', expected=204)
            assert sql('SELECT id FROM identities WHERE id=?', (target,)) == []
            assert sql('SELECT position FROM progress WHERE identity_id=10002') == [(3.0,)]
            assert sql('SELECT cfi FROM annotations WHERE identity_id=10002') == [('other',)]
            sql('DELETE FROM annotations WHERE media_id=?', (media_id,))
            sql('DELETE FROM progress WHERE media_id=?', (media_id,))
            saved_thumbnail = sql('SELECT thumb_path FROM media WHERE id=?', (media_id,))[0][0]
            thumbnail, thumbnail_bytes = qualified_thumbnail(data, media_id, saved_thumbnail)
            retained_artwork = {data / 'thumbs' / f'{media_id}.jpg': b'owned legacy artwork sentinel',
                                data / 'thumbs' / 'unknown.jpg': b'owned unknown artwork sentinel'}
            for path, content in retained_artwork.items():
                with path.open('xb') as artifact:
                    artifact.write(content)
            sql("CREATE TRIGGER qa_media_failure BEFORE DELETE ON media BEGIN SELECT RAISE(ABORT,'owned QA'); END")
            delete_media(media_id, token, 500)
            assert sql('SELECT path,thumb_path FROM media WHERE id=?', (media_id,)) == [('/roots/a/first.cbz', saved_thumbnail)]
            for source in (roots / 'a/first.cbz', roots / 'b/second.cbz'):
                assert source.read_bytes() == source_bytes
            assert thumbnail.read_bytes() == thumbnail_bytes
            for path, content in retained_artwork.items():
                assert path.read_bytes() == content
            sql('DROP TRIGGER qa_media_failure')
            delete_media(media_id, token, 204)
            assert sql('SELECT id FROM media WHERE id=?', (media_id,)) == []
            assert not thumbnail.exists() and not thumbnail.is_symlink(), 'selected UUID thumbnail was not removed'
            for source in (roots / 'a/first.cbz', roots / 'b/second.cbz'):
                assert source.read_bytes() == source_bytes
            for path, content in retained_artwork.items():
                assert path.read_bytes() == content
            print('PASS: admin deletes: guards 409/403/404, failures roll back (500), thumbnail removed after commit')
            # An already configured removable/NAS root becoming unavailable must not
            # destroy HTTP/admin access or turn the stored snapshot into an empty list.
            # New writes still use the strict missing-path 400 oracle above.
            # Normalize the expected rediscovery of the source-preserving deletion
            # before taking the separate unavailable-root snapshot.
            request('/api/scan', token=token, method='POST', expected=202)
            wait_for_scan_idle()
            offline_id = ids['/roots/b/second.cbz']
            request('/api/progress', {'media_id': offline_id, 'position': 0.75}, token, expected=204, method='POST')
            request('/api/annotations', {'media_id': offline_id, 'kind': 'bookmark', 'spine_idx': 0,
                                         'cfi': 'qa-offline-root'}, token, expected=201, method='POST')
            before_offline = sql('SELECT id,path,title FROM media ORDER BY id')
            offline = roots / 'b-offline'
            (roots / 'b').rename(offline)
            try:
                restart_app(compose)
                assert json.loads(request('/api/health')) == {'status': 'ok'}
                assert settings()['effective_library_paths'] == ['/roots/a', '/roots/b']
                raw_status = request('/api/scan/status')
                status = json.loads(raw_status)
                assert status['library_configured'] is True and status['library_count'] == 2
                assert status['scanning'] is False and status['scan_error'] == 'Library root unavailable'
                assert all(path not in raw_status.decode() for path in ('/roots/a', '/roots/b')), 'scan status exposed a configured root path'
                assert sql('SELECT id,path,title FROM media ORDER BY id') == before_offline
                assert offline_id in {item['id'] for item in library()}
                assert json.loads(request(f'/api/progress/{offline_id}', token=token))['position'] == 0.75
                assert len(json.loads(request(f'/api/annotations/{offline_id}', token=token))) == 1
                assert request(f'/api/stream/{queued_id}') == download_bytes
                assert not (roots / 'b').exists(), 'startup silently recreated unavailable root'
                assert (offline / 'second.cbz').read_bytes() == source_bytes
                before_offline_write = settings()
                # Retaining the exact stored offline root permits other settings edits.
                post_settings({'library_paths': ['/roots/a', '/roots/b']}, 204)
                assert settings() == before_offline_write
                # A genuinely new missing root is still strict, with atomic rejection.
                assert not (roots / 'new-missing').exists()
                post_settings({'library_paths': ['/roots/a', '/roots/b', '/roots/new-missing'],
                               'site_name': 'must-not-commit'}, 400)
                assert settings() == before_offline_write
                assert not (roots / 'b').exists(), 'settings silently recreated unavailable root'
            finally:
                offline.rename(roots / 'b')
            request('/api/scan', token=token, method='POST', expected=202)
            wait_for_scan_idle()
            assert scan_status()['scan_error'] is None
            restart_app(compose)
            assert scan_status()['scan_error'] is None
            assert request(f'/api/stream/{offline_id}') == source_bytes
            assert json.loads(request(f'/api/progress/{offline_id}', token=token))['position'] == 0.75
            assert sql('SELECT id,path,title FROM media ORDER BY id') == before_offline
            print('PASS: unavailable library root keeps HTTP, rows and progress; recovers after return, rescan and restart')
    assert_project_removed(project)
    print(f'PASS: {project} resources removed')


def main():
    """Exercise isolated container HTTP, persistence, DNS and read-only media paths."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app-image', default='frayd-qa:local')
    parser.add_argument('--proxy-image', default='frayd-proxy-qa:local')
    args = parser.parse_args()
    project = 'frayd-qa-' + secrets.token_hex(4)
    print(f'QA project: {project}')
    config = qa_config(args, project)
    compose = project_compose(project, config)

    with cleanup_project(compose):
        start_project(compose)
        name = container_id(compose, 'app')
        runtime = json.loads(docker('inspect', name).stdout)[0]
        assert runtime['Config']['User'] == '10001:10001'
        assert runtime['HostConfig']['ReadonlyRootfs'] is True
        assert runtime['HostConfig']['PortBindings'] == {}
        assert runtime['HostConfig']['Memory'] == 2 * 1024**3
        network = next(iter(runtime['NetworkSettings']['Networks']))
        assert json.loads(docker('network', 'inspect', network).stdout)[0]['Internal'] is True
        print('PASS: non-root, read-only root, memory cap, unpublished app, internal network')
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
        print('PASS: first-run setup, duplicate setup rejected, session works')
        fixture = comic_archive()
        docker('exec', '-i', name, 'sh', '-c', 'cat > /data/library/qa-comic.cbz', input=fixture)
        request('/api/scan', {}, token, expected=202)
        wait_for_scan_idle()
        items = json.loads(request('/api/library'))
        assert len(items) == 1 and items[0]['kind'] == 'comic', items
        media_id = items[0]['id']
        pages = json.loads(request(f'/api/comic/{media_id}/pages'))
        assert len(pages) == 1
        assert request(f'/api/comic/{media_id}/page/0') == PNG
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
        print('PASS: byte ranges: bounded, open, suffix and clamped; unsatisfiable and malformed 416; multi-range 200')
        request('/api/progress', {'media_id': media_id, 'position': 0.5}, token, expected=204)
        assert json.loads(request(f'/api/progress/{media_id}', token=token))['position'] == 0.5
        print('PASS: comic scan, page bytes, stream, range stream, progress')
        proxy_name = container_id(compose, 'proxy')
        proxy_before = json.loads(docker('inspect', proxy_name).stdout)[0]
        old_ip = runtime['NetworkSettings']['Networks'][network]['IPAddress']
        compose('stop', 'app')
        compose('rm', '-f', 'app')
        config['services']['reserve'] = {
            'image': args.proxy_image, 'entrypoint': ['sleep', '120'],
            'networks': {'isolated': {}},
            'read_only': True, 'cap_drop': ['ALL'],
        }
        compose('up', '-d', '--no-deps', 'reserve')
        compose('up', '-d', '--no-deps', 'app')
        name = container_id(compose, 'app')
        new_runtime = json.loads(docker('inspect', name).stdout)[0]
        new_ip = new_runtime['NetworkSettings']['Networks'][network]['IPAddress']
        assert new_ip != old_ip, (old_ip, new_ip)
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
        print(f"PASS: app IP {old_ip} -> {new_ip}; proxy {proxy_after['Id'][:12]} (started {proxy_after['State']['StartedAt']}) "
              f'unchanged and recovered in {recovered:.2f}s; query, range and WebSocket preserved')
        compose('rm', '-s', '-f', 'reserve')
        del config['services']['reserve']
        compose('down')
        start_project(compose)
        assert json.loads(request('/api/setup')) == {'needed': False}
        login = json.loads(request('/api/auth/login', credentials))
        token = login['token']
        assert json.loads(request(f'/api/progress/{media_id}', token=token))['position'] == 0.5
        request('/api/auth/logout', {}, token, expected=204)
        request('/api/auth/me', token=token, expected=401)
        print('PASS: recreation keeps setup and progress; login and logout work')
        name = container_id(compose, 'app')
        with tempfile.TemporaryDirectory(prefix=project + '-') as directory:
            media = Path(directory)
            media.chmod(0o755)
            (media / 'qa-comic.cbz').write_bytes(fixture)
            video = synthetic_video(name, '/tmp/qa-video.mkv', 'black', 16, 0.5, 440)
            (media / 'qa-video.mkv').write_bytes(video)
            wrapper = media / 'qa-ffprobe'
            wrapper.write_text('#!/bin/sh\ntouch /tmp/qa-ffprobe-entered\n'
                               'while [ ! -e /tmp/qa-ffprobe-release ]; do sleep 0.1; done\n'
                               'exec /usr/bin/ffprobe "$@"\n')
            wrapper.chmod(0o755)
            media_names = {path.name for path in media.iterdir()}
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
            start_project(compose)
            name = container_id(compose, 'app')
            assert request(f'/api/stream/{media_id}') == fixture
            docker('exec', name, 'sh', '-c', 'test ! -w /data/library/qa-comic.cbz')
            print('PASS: read-only media mount serves the fixture bytes')
            # Recreated apps automatically scan; the wrapper must block that startup scan.
            assert json.loads(request('/api/scan/status', timeout=2))['scanning'] is True
            request('/api/scan', {}, token, expected=409)
            wait_until(lambda: docker('exec', name, 'sh', '-c',
                                      'if test -e /tmp/qa-ffprobe-entered; then echo entered; fi').stdout.strip(),
                       'scan did not enter blocked ffprobe', 5)
            assert json.loads(request('/api/scan/status', timeout=2))['scanning'] is True
            check_scan_read_paths('DURING blocked scan', media_id, token)
            docker('exec', name, 'touch', '/tmp/qa-ffprobe-release')
            wait_for_scan_idle()
            check_scan_read_paths('AFTER released scan', media_id, token)
            movies = [item for item in json.loads(request('/api/library'))
                      if item['title'] == 'qa-video' and item['kind'] == 'movie']
            assert len(movies) == 1, movies
            print('PASS: scan completes once ffprobe is released')
            movie_id = movies[0]['id']
            outputs = wait_until(lambda: transcode_outputs(name),
                                 'real ffmpeg conversion did not produce cached output', 30, 0.5)
            assert len(outputs) == 1, outputs
            output = Path(outputs[0])
            assert output.parent.parent == Path('/data/transcodes') / str(movie_id), output
            uuid.UUID(output.parent.name)
            codecs = json.loads(docker('exec', name, '/usr/bin/ffprobe', '-v', 'error',
                                       '-show_entries', 'stream=codec_name', '-of', 'json', str(output)).stdout)
            assert {stream['codec_name'] for stream in codecs['streams']} == {'h264', 'aac'}, codecs
            converted = docker('exec', name, 'cat', str(output)).stdout
            assert converted and converted != video
            wait_until(lambda: request(f'/api/stream/{movie_id}') == converted,
                       'completed conversion path was not published to HTTP streaming', 4)
            assert (media / 'qa-video.mkv').read_bytes() == video
            assert {path.name for path in media.iterdir()} == media_names
            compose('restart', 'app')
            ready()
            assert request(f'/api/stream/{movie_id}') == converted
            print(f'PASS: H264/AC3 converted to H264/AAC at {output}; source untouched; bytes survive restart')
            docker('exec', name, 'touch', '/tmp/qa-ffprobe-release')
            wait_for_scan_idle()
            request('/api/progress', {'media_id': movie_id, 'position': 0.25}, token, expected=204)
            assert json.loads(request(f'/api/progress/{movie_id}', token=token))['position'] == 0.25
            old_outputs = set(transcode_outputs(name))
            replacement_video = synthetic_video(name, '/tmp/qa-replacement.mkv', 'blue', 32, 1.5, 880)
            assert replacement_video != video and len(replacement_video) != len(video)
            source = media / 'qa-video.mkv'
            old_stat = source.stat()
            source.write_bytes(replacement_video)
            os.utime(source, ns=(old_stat.st_atime_ns, old_stat.st_mtime_ns + 5_000_000_000))
            assert source.stat().st_size != old_stat.st_size
            assert source.stat().st_mtime_ns > old_stat.st_mtime_ns
            request('/api/scan', {}, token, expected=202)
            wait_for_scan_idle()
            assert request(f'/api/stream/{movie_id}', timeout=2) != converted, (
                f'rescan served stale pre-replacement converted bytes for media {movie_id}')
            assert json.loads(request(f'/api/progress/{movie_id}', token=token))['position'] == 0.25
            new_outputs = wait_until(lambda: set(transcode_outputs(name)) - old_outputs,
                                     'replacement source did not produce a new conversion UUID', 30, 0.5)
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
            wait_until(lambda: request(f'/api/stream/{movie_id}', timeout=2) == replacement_converted,
                       'replacement conversion was not published to HTTP streaming', 4)
            assert source.read_bytes() == replacement_video
            assert {path.name for path in media.iterdir()} == media_names
            docker('exec', name, 'sh', '-c', 'test ! -w /data/library/qa-video.mkv')
            replacement_items = [item for item in json.loads(request('/api/library')) if item['title'] == 'qa-video']
            assert len(replacement_items) == 1 and replacement_items[0]['id'] == movie_id
            print(f'PASS: replaced source gets a new cache at {replacement_output}; id and progress kept')
            compose('restart', 'app')
            ready()
            docker('exec', name, 'touch', '/tmp/qa-ffprobe-release')
            wait_for_scan_idle()
            assert request(f'/api/stream/{movie_id}') == replacement_converted
            assert json.loads(request(f'/api/progress/{movie_id}', token=token))['position'] == 0.25
            assert source.read_bytes() == replacement_video
            assert {path.name for path in media.iterdir()} == media_names
            assert any(item['id'] == movie_id for item in json.loads(request('/api/library')))
            print('PASS: replacement bytes, id and progress survive restart')
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
            library_names = {'Film.mkv', 'Film.browser.mp4', 'qa-book.epub', 'qa-cache-failure.cbz'}
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
            config['services']['app']['volumes'] = [
                {'type': 'bind', 'source': str(data), 'target': '/data',
                 'bind': {'create_host_path': False}},
                {'type': 'bind', 'source': str(library), 'target': '/data/library',
                 'read_only': True, 'bind': {'create_host_path': False}},
            ]
            with cleanup_project(compose):
                start_project(compose)
                name = container_id(compose, 'app')
                wait_for_scan_idle()
                assert legacy_fixture_snapshot(database) == before_legacy
                assert request('/api/stream/2') == converted
                assert json.loads(request('/api/progress/2?identity=qa-legacy-device'))['position'] == 42
                genuine_source.write_bytes(replacement_converted)
                os.utime(genuine_source, (timestamp + 5, timestamp + 5))
                request('/api/scan', {}, expected=202)
                wait_for_scan_idle()
                after_legacy = legacy_fixture_snapshot(database)
                assert len(after_legacy['media']) == 2
                assert after_legacy['media'][0] == before_legacy['media'][0]
                assert after_legacy['progress'] == before_legacy['progress']
                assert legacy_source.read_bytes() == video
                assert genuine_source.read_bytes() == replacement_converted
                assert {path.name for path in library.iterdir()} == library_names
                assert request('/api/stream/2') == replacement_converted
                assert json.loads(request('/api/progress/2?identity=qa-legacy-device'))['position'] == 42
                genuine_row = after_legacy['media'][1]
                assert genuine_row['size_bytes'] == len(replacement_converted), (
                    f"legacy-referenced genuine media2 stored size {genuine_row['size_bytes']} != actual {len(replacement_converted)} after scan")
                expected_genuine = {**before_legacy['media'][1], 'size_bytes': len(replacement_converted), 'mtime_secs': timestamp + 5}
                assert genuine_row == expected_genuine
                docker('exec', name, 'sh', '-c', 'test ! -w /data/library/Film.browser.mp4')
                print(f"PASS: legacy-referenced media 2 size {before_legacy['media'][1]['size_bytes']}->{genuine_row['size_bytes']}, "
                      'mtime refreshed; ids, titles, progress, source hashes and legacy pointer kept')
                restart_app(compose)
                assert legacy_fixture_snapshot(database) == after_legacy
                assert request('/api/stream/2') == replacement_converted
                assert json.loads(request('/api/progress/2?identity=qa-legacy-device'))['position'] == 42
                assert legacy_source.read_bytes() == video
                assert genuine_source.read_bytes() == replacement_converted
                print('PASS: legacy rows, progress and source bytes survive restart')
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
                request('/api/scan', {}, expected=202)
                wait_for_scan_idle()
                invalidated_ebook = epub_fixture_snapshot(database)
                expected_ebook = {
                    'media': {**warm_ebook['media'], 'size_bytes': len(new_epub), 'mtime_secs': timestamp + 5},
                    'metadata': ebook_metadata,
                    'progress': warm_ebook['progress'],
                }
                assert 'spine' not in invalidated_ebook['metadata'], 'EPUB rescan retained stale cached spine'
                assert invalidated_ebook == expected_ebook
                assert json.loads(request('/api/epub/3/spine')) == new_spine
                assert request('/api/epub/3/chunk/0') == new_chapters[0][1]
                assert request('/api/epub/3/chunk/1') == new_chapters[1][1]
                request('/api/epub/3/chunk/2', expected=404)
                refreshed_ebook = epub_fixture_snapshot(database)
                assert refreshed_ebook == {**expected_ebook, 'metadata': {**ebook_metadata, 'spine': new_spine}}
                assert json.loads(request('/api/progress/3?identity=qa-legacy-device')) == {'position': 1.0, 'updated_at': 12346}
                assert ebook_source.read_bytes() == new_epub
                assert legacy_source.read_bytes() == video
                assert genuine_source.read_bytes() == replacement_converted
                assert {path.name for path in library.iterdir()} == library_names
                print('PASS: EPUB rescan drops the stale spine; new chapters served, removed chapter 404; progress kept')
                restart_app(compose)
                assert epub_fixture_snapshot(database) == refreshed_ebook
                assert request('/api/epub/3/chunk/0') == new_chapters[0][1]
                request('/api/epub/3/chunk/2', expected=404)
                assert json.loads(request('/api/progress/3?identity=qa-legacy-device'))['position'] == 1
                assert ebook_source.read_bytes() == new_epub
                print('PASS: EPUB metadata, chapters and progress survive restart')
                comic_before = comic_cache_failure_snapshot(database)
                assert json.loads(comic_before['media']['meta_json']) == comic_metadata
                assert comic_before['progress']['position'] == 0
                assert comic_source.read_bytes() == fixture
                docker('exec', name, 'sh', '-c', 'test -r /data/library/qa-cache-failure.cbz && test ! -w /data/library/qa-cache-failure.cbz')
                assert json.loads(request('/api/comic/4/pages')) == ['001.png']
                assert request('/api/comic/4/page/0') == PNG
                request('/api/comic/4/page/1', expected=404)
                assert comic_cache_failure_snapshot(database) == comic_before
                assert json.loads(request('/api/progress/4?identity=qa-legacy-device')) == {'position': 0.0, 'updated_at': 12347}
                assert comic_source.read_bytes() == fixture
                assert {path.name for path in library.iterdir()} == library_names
                print('PASS: comic served (200) despite a cache write error; page 1 is 404; rows unchanged')
                restart_app(compose)
                assert json.loads(request('/api/comic/4/pages')) == ['001.png']
                assert request('/api/comic/4/page/0') == PNG
                assert comic_cache_failure_snapshot(database) == comic_before
                assert comic_source.read_bytes() == fixture
                print('PASS: comic readable after restart; cache trigger still rejects writes; rows unchanged')

    assert_project_removed(project)
    print(f'PASS: {project} resources removed')
    upstream_contract_qualification(args)


if __name__ == '__main__':
    main()
