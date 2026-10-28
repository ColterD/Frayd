"""Disposable HTTP user-path QA; credentials stay only in process memory."""
import base64
import io
import json
import secrets
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import zipfile
from pathlib import Path

root = Path(__file__).resolve().parents[1]
base = ''

def docker(*args, **kwargs):
    return subprocess.run(['docker', *args], cwd=root, check=True, capture_output=True, **kwargs)

def request(path, data=None, token=None, headers=None, expected=200):
    fields = dict(headers or {})
    if token:
        fields['Authorization'] = 'Bearer ' + token
    if data is not None:
        fields['Content-Type'] = 'application/json'
    req = urllib.request.Request(base + path, data=None if data is None else json.dumps(data).encode(), headers=fields)
    try:
        response = urllib.request.urlopen(req, timeout=15)
    except urllib.error.HTTPError as error:
        assert error.code == expected, (path, error.code, expected)
        return b''
    assert response.status == expected, (path, response.status, expected)
    return response.read()

def ready():
    for _ in range(40):
        try:
            request('/api/setup')
            return
        except urllib.error.URLError:
            time.sleep(0.5)
    raise AssertionError('QA server did not become ready')

project = 'frayd-qa-' + secrets.token_hex(4)
config = json.loads(docker('compose', '-f', str(root / 'compose.yaml'), 'config', '--format', 'json').stdout)
config['name'] = project
for key, network in config['networks'].items():
    network['name'] = project + '_' + key
for key, volume in config['volumes'].items():
    volume['name'] = project + '_' + key
for service in config['services'].values():
    service.pop('build', None)
config['services']['proxy']['ports'][0]['published'] = '0'
config_data = json.dumps(config).encode()

def compose(*args):
    return docker('compose', '-p', project, '-f', '-', *args, input=config_data)

try:
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
    with tempfile.TemporaryDirectory(prefix=project + '-') as directory:
        media = Path(directory)
        media.chmod(0o755)
        (media / 'qa-comic.cbz').write_bytes(fixture)
        config['services']['app']['volumes'].append({
            'type': 'bind', 'source': directory, 'target': '/data/library',
            'read_only': True, 'bind': {'create_host_path': False},
        })
        config_data = json.dumps(config).encode()
        compose('up', '-d', '--wait', '--wait-timeout', '90')
        base = 'http://' + compose('port', 'proxy', '8080').stdout.decode().strip()
        name = compose('ps', '-q', 'app').stdout.decode().strip()
        assert request(f'/api/stream/{media_id}') == fixture
        docker('exec', name, 'sh', '-c', 'test ! -w /data/library/qa-comic.cbz')
        print('PASS: documented read-only media bind mount serves exact fixture bytes')
        compose('down')
finally:
    compose('down', '--volumes')
    print('QA project containers, networks and named volume removed')
