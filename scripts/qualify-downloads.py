"""Qualify opt-in Rust direct-file downloads on one disposable app image and owned HTTP fixtures."""
import argparse
import copy
import hashlib
import json
import secrets
import re
import shutil
import subprocess
import tempfile
from pathlib import Path
from urllib.parse import urlsplit
import urllib.request

import integration as qa

if not __debug__:
    raise SystemExit('unsupported optimized Python execution: qualification requires assertions; remove -O and PYTHONOPTIMIZE')

ROOT = Path(__file__).resolve().parents[1]
# The checked-out Git blob is the oracle; the public commit URL is supplied explicitly by the caller/CI.
FIXTURE_REF = 'HEAD:tests/fixtures/native-resume.mp4'
FIXTURE_READ_LIMIT = 1024 * 1024


def ingest(url, token, expected=202):
    qa.request('/api/ingest', {'url': url}, token, expected=expected)


def finish(expected):
    def terminal():
        status = qa.scan_status()['ingesting']
        return status if status in ('done', 'failed') else None
    observed = qa.wait_until(terminal, 'download did not reach terminal state', 90, 0.25)
    assert observed == expected, (observed, expected)
    qa.wait_for_scan_idle()


def owner():
    return json.loads(qa.request('/api/setup', {
        'email': 'downloads-qa@example.invalid', 'password': secrets.token_urlsafe(32)}))['token']


def assert_profile_contract(default, optional):
    """The opt-in cannot change images/builds, published ports, storage or security settings."""
    expected = copy.deepcopy(default)
    expected['services']['app']['environment'].update(FRAYD_DOWNLOADS_ENABLED='1', FRAYD_DOWNLOAD_BACKEND='direct')
    expected['services']['app']['networks']['downloads-egress'] = None
    expected['networks']['downloads-egress'] = {'name': default['name'] + '_downloads-egress'}
    actual = copy.deepcopy(optional)
    # Compose versions may emit an empty IPAM map for a newly declared default bridge.
    assert actual['networks']['downloads-egress'].pop('ipam', None) in (None, {})
    assert expected == actual, 'Opt-in must change only download enablement/backend and app egress'


def controlled_config(args, project, optional):
    config = qa.qa_config(args, project)
    if optional:
        effective = json.loads(qa.docker('compose', '-f', str(ROOT / 'compose.yaml'), '-f',
                                        str(ROOT / 'compose.downloads.yaml'), 'config', '--format', 'json').stdout)
        config['services']['app']['networks'] = effective['services']['app']['networks']
        config['networks']['downloads-egress'] = {'name': project + '_downloads-egress'}
        config['services']['app']['environment'].update(effective['services']['app']['environment'])
    for service in config['services'].values():
        service['restart'] = 'no'
    return config


def fixture_service(args, network, directory=None):
    """Reuse the existing proxy image as a controlled HTTP origin; no new app runtime dependency."""
    configuration = """pid /tmp/fixture.pid;
error_log /dev/stderr warn;
events { worker_connections 64; }
http {
    access_log off;
    client_body_temp_path /tmp/client_body;
    proxy_temp_path /tmp/proxy;
    fastcgi_temp_path /tmp/fastcgi;
    uwsgi_temp_path /tmp/uwsgi;
    scgi_temp_path /tmp/scgi;
    types { video/mp4 mp4; text/html html; }
    server { listen 8000; root /fixtures; location = / { return 200 'owned fixture'; } }
}
"""
    service = {
        'image': args.proxy_image,
        'entrypoint': ['sh', '-c', "cat > /tmp/fixture.conf <<'FRAYD_FIXTURE'\n" + configuration
                       + "FRAYD_FIXTURE\nexec nginx -c /tmp/fixture.conf -g 'daemon off;'"],
        'user': '10001:10001', 'read_only': True, 'cap_drop': ['ALL'],
        'security_opt': ['no-new-privileges:true'], 'pids_limit': 64, 'mem_limit': '128m',
        'tmpfs': ['/tmp:rw,noexec,nosuid,size=32m'], 'networks': [network],
        'healthcheck': {'test': ['CMD', 'wget', '-q', '--spider', 'http://127.0.0.1:8000/'], 'interval': '1s'},
    }
    if directory is not None:
        service['volumes'] = [{'type': 'bind', 'source': directory, 'target': '/fixtures', 'read_only': True}]
    return service


def security(container):
    info = json.loads(qa.docker('inspect', container).stdout)[0]
    host = info['HostConfig']
    assert info['Config']['User'] == '10001:10001'
    assert host['ReadonlyRootfs'] and host['CapDrop'] == ['ALL']
    assert host['SecurityOpt'] == ['no-new-privileges:true']
    assert host['Memory'] == 2 * 1024**3 and host['NanoCpus'] == 2 * 10**9
    assert not host['PortBindings']
    return info


PUBLIC_ASSETS = ('index.html', 'css/frayd.css', 'js/app.js', 'js/party.js', 'js/build-info.js',
                 'vendor/plyr/plyr.min.js', 'vendor/plyr/plyr.min.css', 'vendor/plyr/plyr.svg')


def public_assets(container):
    """Qualify actual public HTTP bytes and root-owned readable, non-writable asset modes."""
    for path in PUBLIC_ASSETS:
        assert qa.request('/' if path == 'index.html' else '/' + path) == (ROOT / 'client' / path).read_bytes(), path
    metadata = qa.docker('exec', container, 'find', '/app/client', '-printf', '%y %m %U %G %p\n').stdout.decode().splitlines()
    files = set()
    for line in metadata:
        kind, mode, uid, gid, path = line.split(' ', 4)
        assert uid == gid == '0', line
        assert mode == ('755' if kind == 'd' else '644'), line
        if kind == 'f':
            files.add(path.removeprefix('/app/client/'))
        else:
            assert kind == 'd', line
    assert files == set(PUBLIC_ASSETS), files
    print('PASS: index and all7 nested assets HTTP200 with exact source bytes; runtime dirs0755/files0644/root-owned')


def retire_egress(project):
    """Compose can retain an unused bridge after rollback; retire only this exact empty fixture bridge."""
    name = project + '_downloads-egress'
    remaining = qa.docker('network', 'ls', '-q', '--filter', f'label=com.docker.compose.project={project}',
                          '--filter', f'name=^{name}$').stdout.decode().splitlines()
    assert len(remaining) <= 1, remaining
    for identity in remaining:
        network = json.loads(qa.docker('network', 'inspect', identity).stdout)[0]
        assert network['Name'] == name
        assert network['Labels'].get('com.docker.compose.project') == project
        assert not network.get('Containers'), 'fixture egress bridge still has connected containers'
        qa.docker('network', 'rm', identity)


def disabled_scenario(args, enabled_without_tool=False, invalid_backend=False):
    project = 'frayd-downloads-disabled-' + secrets.token_hex(4)
    config = controlled_config(args, project, False)
    if enabled_without_tool:
        config['services']['app']['environment']['FRAYD_DOWNLOADS_ENABLED'] = '1'
        config['services']['app']['environment']['FRAYD_DOWNLOAD_BACKEND'] = 'yt-dlp'
    elif invalid_backend:
        config['services']['app']['environment'].update(FRAYD_DOWNLOADS_ENABLED='1', FRAYD_DOWNLOAD_BACKEND='invalid-qualification-backend')
    # An owned, healthy service on the egress-only edge makes the refusal oracle independent of remote availability.
    config['services']['egress-fixture'] = fixture_service(args, 'edge')
    compose = qa.project_compose(project, config)
    with qa.cleanup_project(compose):
        qa.start_project(compose)
        container = qa.container_id(compose, 'app')
        info = security(container)
        if not enabled_without_tool and not invalid_backend:
            public_assets(container)
        token = owner()
        settings = json.loads(qa.request('/api/admin/settings', token=token))
        assert settings['downloads_enabled'] is enabled_without_tool
        assert settings['download_backend'] == (None if invalid_backend else 'yt-dlp' if enabled_without_tool else 'direct')
        if invalid_backend:
            assert isinstance(settings['download_config_error'], str) and settings['download_config_error']
            assert '/data' not in settings['download_config_error']
        else:
            assert settings['download_config_error'] is None
        qa.docker('exec', container, 'sh', '-c', '! command -v yt-dlp')
        assert len(info['NetworkSettings']['Networks']) == 1
        network = next(iter(info['NetworkSettings']['Networks']))
        assert json.loads(qa.docker('network', 'inspect', network).stdout)[0]['Internal']
        fixture = qa.container_id(compose, 'egress-fixture')
        fixture_info = json.loads(qa.docker('inspect', fixture).stdout)[0]
        assert set(fixture_info['NetworkSettings']['Networks']) == {project + '_edge'}
        address = fixture_info['NetworkSettings']['Networks'][project + '_edge']['IPAddress']
        assert address and address != info['NetworkSettings']['Networks'][network]['IPAddress']
        proxy = qa.container_id(compose, 'proxy')
        qa.docker('exec', proxy, 'wget', '-q', '-T', '3', '-O', '/dev/null', f'http://{address}:8000/')
        qa.docker('exec', container, 'sh', '-c',
                  'if curl --noproxy "*" --connect-timeout 2 --max-time 4 --silent "$1" >/dev/null; then exit 1; fi',
                  'fixture-isolation', f'http://{address}:8000/')
        before = qa.scan_status()['ingesting']
        ingest('file:///refused', token, 400)
        ingest('https://example.invalid/refused', None, 403)
        ingest('https://example.invalid/refused', token, 501)
        assert qa.scan_status()['ingesting'] == before
        qa.docker('exec', container, 'sh', '-c', 'test ! -e /data/library/Internet')
        print('PASS: ' + ('invalid backend fail-closed' if invalid_backend else 'enabled missing-tool' if enabled_without_tool else 'disabled default') +
              ' metadata/auth/URL/501 guards, no job/root effects, healthy owned edge fixture refused, internal-only network, security retained')
    qa.assert_project_removed(project)


def optional_scenario(args):
    project = 'frayd-downloads-enabled-' + secrets.token_hex(4)
    config = controlled_config(args, project, True)
    with tempfile.TemporaryDirectory(prefix=project) as directory:
        fixtures = Path(directory)
        fixtures.chmod(0o755)
        config['services']['fixture'] = fixture_service(args, 'isolated', directory)
        compose = qa.project_compose(project, config)
        try:
            qa.start_project(compose)
            container = qa.container_id(compose, 'app')
            info = security(container)
            assert info['Image'] == args.app_image
            assert len(info['NetworkSettings']['Networks']) == 2
            networks = [json.loads(qa.docker('network', 'inspect', name).stdout)[0]
                        for name in info['NetworkSettings']['Networks']]
            assert sorted(network['Internal'] for network in networks) == [False, True]
            qa.docker('exec', container, 'sh', '-c', '! command -v yt-dlp && ! command -v python3 && test ! -e /opt/yt-dlp')
            print('PASS: one unchanged app image; Rust direct backend enabled only through explicit environment and egress')
            # Generate in the container; the host needs no codec tools or Python installations.
            qa.docker('exec', container, 'ffmpeg', '-v', 'error', '-f', 'lavfi', '-i',
                      'color=c=blue:s=64x64:r=10:d=2', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '/tmp/source.mp4')
            video = qa.docker('exec', container, 'cat', '/tmp/source.mp4').stdout
            (fixtures / 'owned.mp4').write_bytes(video)
            qa.docker('exec', container, 'ffmpeg', '-v', 'error', '-i', '/tmp/source.mp4', '-c', 'copy', '-f', 'matroska', '/tmp/not-mp4.mkv')
            (fixtures / 'matroska.mp4').write_bytes(qa.docker('exec', container, 'cat', '/tmp/not-mp4.mkv').stdout)
            qa.docker('exec', container, 'ffmpeg', '-v', 'error', '-f', 'lavfi', '-i',
                      'sine=frequency=440:duration=1', '-c:a', 'aac', '/tmp/audio-only.mp4')
            (fixtures / 'audio-only.mp4').write_bytes(qa.docker('exec', container, 'cat', '/tmp/audio-only.mp4').stdout)
            (fixtures / 'invalid.mp4').write_bytes(b'not a video')
            (fixtures / 'watch.html').write_text('<!doctype html><video src="owned.mp4"></video>')
            # These exact synthetic public files must remain readable by the UID10001 origin under umask077 too.
            for name in ('owned.mp4', 'matroska.mp4', 'audio-only.mp4', 'invalid.mp4', 'watch.html'):
                (fixtures / name).chmod(0o644)
            token = owner()
            settings = json.loads(qa.request('/api/admin/settings', token=token))
            assert settings['downloads_enabled'] is True and settings['download_backend'] == 'direct'
            assert settings['download_config_error'] is None
            qa.request('/api/admin/settings', {'downloads_enabled': False}, token, expected=400)
            qa.request('/api/admin/settings', {'download_backend': 'yt-dlp'}, token, expected=400)
            member = json.loads(qa.request('/api/auth/login', {
                'email': 'member-qa@example.invalid', 'password': secrets.token_urlsafe(32)}))
            assert member['is_admin'] is False
            ingest('http://fixture:8000/owned.mp4', member['token'], 403)
            ingest('http://fixture:8000/owned.mp4', None, 403)
            ingest('file:///refused', token, 400)
            ingest('http://fixture:8000/invalid.mp4', token)
            finish('failed')
            ingest('http://fixture:8000/missing.mp4', token)
            finish('failed')
            ingest('http://fixture:8000/watch.html', token)
            finish('failed')
            ingest('http://fixture:8000/matroska.mp4', token)
            finish('failed')
            ingest('http://fixture:8000/audio-only.mp4', token)
            finish('failed')
            assert json.loads(qa.request('/api/library')) == []
            ingest('http://fixture:8000/owned.mp4', token)
            finish('done')
            items = json.loads(qa.request('/api/library'))
            assert len(items) == 1 and items[0]['kind'] in ('movie', 'episode', 'video'), items
            media_id = items[0]['id']
            stream = qa.request(f'/api/stream/{media_id}')
            assert stream == video
            published = qa.docker('exec', container, 'find', '/data/library/Internet', '-type', 'f').stdout.decode().splitlines()
            assert len(published) == 1 and published[0].endswith('.mp4'), published
            source = published[0]
            qa.docker('exec', container, 'ffmpeg', '-v', 'error', '-i', source, '-f', 'null', '-')
            subprocess.run(['node', str(ROOT / 'scripts/qualify-download-playback.cjs'),
                            qa.base + f'/api/stream/{media_id}'], check=True, timeout=45)
            # Replace upstream bytes under the same filename; a second valid download cannot replace the library file.
            qa.docker('exec', container, 'ffmpeg', '-v', 'error', '-f', 'lavfi', '-i',
                      'color=c=red:s=64x64:r=10:d=2', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '/tmp/changed.mp4')
            changed = qa.docker('exec', container, 'cat', '/tmp/changed.mp4').stdout
            assert changed != video
            (fixtures / 'owned.mp4').write_bytes(changed)
            ingest('http://fixture:8000/owned.mp4', token)
            finish('failed')
            assert qa.docker('exec', container, 'cat', source).stdout == video
            assert qa.request(f'/api/stream/{media_id}') == video
            qa.docker('exec', container, 'sh', '-c',
                      'test -z "$(find /data/library -name \'*.part\')" && test -z "$(find /data/transcodes -maxdepth 1 -name \'ingest-*\')"')
            print(f'PASS: real Rust direct HTTP/API/index/stream/decode; invalid/404/HTML/nonMP4/audio-only/nonadmin guards; no overwrite; sha256={hashlib.sha256(video).hexdigest()}')
            # Restore default environment/routing on the SAME image and volume; keep persisted data.
            compose('stop')
            config['services'].pop('fixture')
            config['services']['app']['image'] = args.app_image
            config['services']['app']['environment'] = qa.qa_config(args, project)['services']['app']['environment']
            config['services']['app']['networks'] = {'isolated': None}
            qa.start_project(compose)
            qa.wait_for_scan_idle()
            rolled_back = security(qa.container_id(compose, 'app'))
            assert rolled_back['Image'] == info['Image'] == args.app_image
            assert len(rolled_back['NetworkSettings']['Networks']) == 1
            before_volume = next(mount['Name'] for mount in info['Mounts'] if mount['Destination'] == '/data')
            after_volume = next(mount['Name'] for mount in rolled_back['Mounts'] if mount['Destination'] == '/data')
            assert after_volume == before_volume
            assert qa.request(f'/api/stream/{media_id}') == video
            assert json.loads(qa.request('/api/admin/settings', token=token))['downloads_enabled'] is False
            ingest('http://fixture:8000/owned.mp4', token, 501)
            print('PASS: rollback to default environment/network on SAME image preserves volume, indexed video and owner session')
        finally:
            # Never delete volumes during rollback. Only this disposable qualifier's exact volume is removed here.
            compose('down', '--remove-orphans')
            volume = config['volumes']['data']['name']
            qa.docker('volume', 'rm', volume)
            retire_egress(project)
        qa.assert_project_removed(project)
    print('PASS: exact disposable project resources removed')


def private_assets_scenario(args):
    """Reproduce a private checkout COPY using the exact production normalization step; no Rust rebuild."""
    project = 'frayd-downloads-private-assets-' + secrets.token_hex(4)
    # Keep an existing caller-owned tag so removing our temporary alias cannot retire an untagged source image.
    original_tags = json.loads(qa.docker('image', 'inspect', '--format', '{{json .RepoTags}}', args.app_image).stdout)
    assert original_tags, 'Private asset QA requires an existing tagged app image; source image must survive cleanup'
    parent_tag = project + ':parent'
    tag = project + ':normalized'
    source = (ROOT / 'Dockerfile').read_text().replace('\\\n', ' ')
    commands = [line.removeprefix('RUN ') for line in source.splitlines() if line.startswith('RUN mkdir -p /app/server')]
    assert len(commands) == 1, 'Public asset preparation must have one explicit production step'
    try:
        qa.docker('tag', args.app_image, parent_tag)
        with tempfile.TemporaryDirectory(prefix=project) as directory:
            context = Path(directory)
            for asset in PUBLIC_ASSETS:
                destination = context / 'client' / asset
                destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                shutil.copyfile(ROOT / 'client' / asset, destination)
                destination.chmod(0o600)
            for folder in (context / 'client').rglob('*'):
                if folder.is_dir():
                    folder.chmod(0o700)
            (context / 'client').chmod(0o700)
            assert all(path.stat().st_mode & 0o777 == (0o700 if path.is_dir() else 0o600)
                       for path in (context / 'client').rglob('*'))
            # Removing only the derived image's public tree gives COPY the same empty destination as production.
            (context / 'Dockerfile').write_text('FROM ' + parent_tag + '\nUSER root\nRUN rm -rf /app/client\n'
                'COPY client/ /app/client/\nRUN ' + commands[0] + '\nUSER 10001:10001\n')
            qa.docker('build', '--network', 'none', '-t', tag, directory)
        fixture_args = copy.copy(args)
        fixture_args.app_image = tag
        config = controlled_config(fixture_args, project, False)
        compose = qa.project_compose(project, config)
        with qa.cleanup_project(compose):
            qa.start_project(compose)
            container = qa.container_id(compose, 'app')
            security(container)
            public_assets(container)
        qa.assert_project_removed(project)
        print('PASS: private COPY source dirs0700/files0600 become root-owned dirs0755/files0644; actual HTTP bytes preserved')
    finally:
        # Only fresh task-owned aliases/images are retired; the caller's immutable app image remains.
        for owned_tag in (tag, parent_tag):
            if qa.docker('image', 'ls', '-q', '--filter', 'reference=' + owned_tag).stdout.strip():
                qa.docker('image', 'rm', '-f', owned_tag)
        assert qa.docker('image', 'inspect', '--format', '{{.Id}}', args.app_image).stdout.decode().strip() == args.app_image


def pinned_fixture():
    """Bound the exact local Git fixture before using it as the independent public HTTPS oracle."""
    size = int(subprocess.run(['git', 'cat-file', '-s', FIXTURE_REF], cwd=ROOT,
                              capture_output=True, text=True, check=True, timeout=15).stdout)
    assert 0 < size <= FIXTURE_READ_LIMIT
    body = subprocess.run(['git', 'cat-file', 'blob', FIXTURE_REF], cwd=ROOT,
                          capture_output=True, check=True, timeout=15).stdout
    assert len(body) == size
    return body


def validate_external_url(url):
    """Permit only a credential-free raw GitHub MP4 fixture at a full immutable commit."""
    parsed = urlsplit(url)
    if (parsed.scheme != 'https' or parsed.hostname != 'raw.githubusercontent.com'
            or parsed.username is not None or parsed.password is not None or parsed.port not in (None, 443)
            or parsed.query or parsed.fragment or not re.fullmatch(
                r'/[A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9][A-Za-z0-9_.-]*/[0-9a-f]{40}/tests/fixtures/native-resume\.mp4',
                parsed.path)):
        raise ValueError('External URL must name the public raw GitHub fixture at a full commit, without credentials/query/fragment')
    return url


def external_tls_scenario(args):
    """One actual Rust HTTPS download; keep external availability separate from controlled qualification."""
    expected = pinned_fixture()
    project = 'frayd-downloads-tls-' + secrets.token_hex(4)
    config = controlled_config(args, project, True)
    compose = qa.project_compose(project, config)
    with qa.cleanup_project(compose):
        qa.start_project(compose)
        container = qa.container_id(compose, 'app')
        security(container)
        token = owner()
        settings = json.loads(qa.request('/api/admin/settings', token=token))
        assert settings['downloads_enabled'] is True and settings['download_backend'] == 'direct'
        assert settings['download_config_error'] is None
        # Never retry this side-effecting POST; polling only observes this one job.
        ingest(args.external_url, token)
        finish('done')
        items = json.loads(qa.request('/api/library'))
        assert len(items) == 1 and items[0]['kind'] in ('movie', 'episode'), items
        with urllib.request.urlopen(qa.base + '/api/stream/' + str(items[0]['id']), timeout=15) as response:
            assert response.status == 200
            actual = response.read(len(expected) + 1)
        assert actual == expected
        print(f'PASS: actual Rust HTTPS MP4 download/index/stream byte-equals immutable public generated fixture; '
              f'bytes={len(actual)} sha256={hashlib.sha256(actual).hexdigest()}; one POST, endpoint-specific proof only')
    qa.assert_project_removed(project)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app-image', default='frayd-qa:local')
    parser.add_argument('--proxy-image', default='frayd-proxy-qa:local')
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--external-tls', action='store_true', help='Run one real Rust HTTPS download of the pinned public generated MP4 fixture')
    mode.add_argument('--private-assets', action='store_true', help='Build a tiny private-source regression image and qualify real asset HTTP paths')
    parser.add_argument('--external-url', help='Explicit public raw GitHub fixture URL at a full commit; required with --external-tls')
    args = parser.parse_args()
    if args.external_tls:
        if args.external_url is None:
            parser.error('--external-tls requires --external-url')
        validate_external_url(args.external_url)
    elif args.external_url is not None:
        parser.error('--external-url requires --external-tls')
    default = json.loads(qa.docker('compose', '-f', str(ROOT / 'compose.yaml'), 'config', '--format', 'json').stdout)
    optional = json.loads(qa.docker('compose', '-f', str(ROOT / 'compose.yaml'), '-f',
                                   str(ROOT / 'compose.downloads.yaml'), 'config', '--format', 'json').stdout)
    assert_profile_contract(default, optional)
    # Resolve tags once so concurrent retagging cannot change a later scenario or rollback.
    for field in ('app_image', 'proxy_image'):
        identity = qa.docker('image', 'inspect', '--format', '{{.Id}}', getattr(args, field)).stdout.decode().strip()
        setattr(args, field, identity)
    print('IMAGES ' + json.dumps(vars(args), sort_keys=True))
    if args.external_tls:
        external_tls_scenario(args)
    elif args.private_assets:
        private_assets_scenario(args)
    else:
        disabled_scenario(args)
        disabled_scenario(args, enabled_without_tool=True)
        disabled_scenario(args, invalid_backend=True)
        optional_scenario(args)
        print('PASS: CONTROLLED qualification only; actual external Rust HTTPS and dependency audit remain separate mandatory readiness gates')


if __name__ == '__main__':
    main()
