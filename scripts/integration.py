"""Disposable HTTP user-path QA; credentials stay only in process memory."""
import argparse
import base64
import hashlib
import io
import json
import os
import secrets
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
import zipfile
from contextlib import contextmanager
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
base = ''

def docker(*args, **kwargs):
    """Run Docker, exposing only its captured stderr when a command fails."""
    try:
        return subprocess.run(['docker', *args], cwd=ROOT, check=True, capture_output=True, **kwargs)
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
        assert error.code == expected, (path, error.code, expected)
        return b''
    assert response.status == expected, (path, response.status, expected)
    return response.read()

def websocket_upgrade(token):
    """Verify a real party WebSocket handshake through the recovered proxy."""
    code = json.loads(request('/api/party', {'identity': 'qa-host'}, token))['code']
    address = urlsplit(base)
    key = base64.b64encode(secrets.token_bytes(16)).decode()
    handshake = (f'GET /api/party/{code}/ws?identity=qa-host HTTP/1.1\r\n'
                 f'Host: {address.netloc}\r\nAuthorization: Bearer {token}\r\n'
                 'Upgrade: websocket\r\nConnection: Upgrade\r\n'
                 f'Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n')
    with socket.create_connection((address.hostname, address.port), timeout=2) as connection:
        connection.sendall(handshake.encode())
        response = b''
        while b'\r\n\r\n' not in response:
            chunk = connection.recv(4096)
            assert chunk and len(response) < 16384, 'incomplete WebSocket handshake'
            response += chunk
    assert response.startswith(b'HTTP/1.1 101 '), response.split(b'\r\n', 1)[0]
    accept = base64.b64encode(hashlib.sha1(
        (key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode(), usedforsecurity=False).digest())
    headers = dict(line.split(b': ', 1) for line in response.split(b'\r\n\r\n', 1)[0].split(b'\r\n')[1:])
    assert next(value for name, value in headers.items() if name.lower() == b'sec-websocket-accept') == accept


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

def wait_for_scan_idle():
    """Wait for scan admission to clear using bounded disposable-app requests."""
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if not json.loads(request('/api/scan/status', timeout=2))['scanning']:
            return
        time.sleep(0.5)
    raise AssertionError('disposable scan did not finish within 30 seconds')


def assert_project_removed(project):
    """Reject any remaining task-owned containers, networks or volumes."""
    for resource in ('container', 'network', 'volume'):
        flags = ('-a',) if resource == 'container' else ()
        remaining = docker(resource, 'ls', '-q', *flags, '--filter', f'label=com.docker.compose.project={project}').stdout.strip()
        assert not remaining, (resource, project, remaining)


def main():
    """Exercise isolated container HTTP, persistence, DNS and read-only media paths."""
    global base
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app-image', default='frayd-qa:local')
    parser.add_argument('--proxy-image', default='frayd-proxy-qa:local')
    args = parser.parse_args()
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
        for _ in range(40):
            if not json.loads(request('/api/scan/status'))['scanning']:
                break
            time.sleep(0.5)
        else:
            raise AssertionError('scan did not finish')
        items = json.loads(request('/api/library'))
        assert len(items) == 1 and items[0]['kind'] == 'comic', items
        media_id = items[0]['id']
        pages = json.loads(request(f'/api/comic/{media_id}/pages'))
        assert len(pages) == 1
        assert request(f'/api/comic/{media_id}/page/0') == image
        assert request(f'/api/stream/{media_id}') == fixture
        assert request(f'/api/stream/{media_id}', headers={'Range': 'bytes=5-'}, expected=206) == fixture[5:]
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
        ready()
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
            for _ in range(60):
                if not json.loads(request('/api/scan/status', timeout=2))['scanning']:
                    break
                time.sleep(0.5)
            else:
                raise AssertionError('released ffprobe scan did not finish')
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
            compose('down')

    assert_project_removed(project)
    print(f'PASS: task-owned {project} containers, networks and volume removed')


if __name__ == '__main__':
    main()
