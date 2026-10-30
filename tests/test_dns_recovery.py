"""Real nginx recovery oracle for one transient DNS SERVFAIL after app replacement."""
import argparse
import functools
import hashlib
import importlib.util
import json
import secrets
import socket
import struct
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PYTHON_IMAGE = 'python@sha256:1b668429b3511ab407d8e00648891631b0b1a4d7e15e3ca70f38ab5b91ad4ab4'
CLEANUP_TIMEOUT = 5


@functools.cache
def harness():
    """Load the shared Docker harness on demand; the container-side --serve mode has only this file."""
    spec = importlib.util.spec_from_file_location('integration', ROOT / 'scripts/integration.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def docker(*args):
    """Run one fixture Docker command under the harness deadline and return its stripped stdout."""
    return harness().docker(*args).stdout.decode().strip()


def write_control(path, state):
    """Publish a complete DNS control document without exposing partial JSON."""
    pending = path.with_suffix('.next')
    pending.write_text(json.dumps(state))
    pending.replace(path)


def serve_dns(control):
    """Return the controlled app A record, with one explicitly armed SERVFAIL."""
    state = Path(control)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as server:
        server.bind((socket.gethostbyname(socket.gethostname()), 53))
        print('DNS ready', flush=True)
        while True:
            packet, peer = server.recvfrom(4096)
            end = 12
            while packet[end]:
                end += packet[end] + 1
            end += 5
            current = json.loads(state.read_text())
            failed = current.get('fail_next', False)
            if failed:
                current['fail_next'] = False
                write_control(state, current)
            flags, answers = (0x8182, 0) if failed else (0x8180, 1)
            response = packet[:2] + struct.pack('!HHHHH', flags, 1, answers, 0, 0) + packet[12:end]
            if not failed:
                response += b'\xc0\x0c' + struct.pack('!HHIH', 1, 1, 1, 4) + socket.inet_aton(current['ip'])
            server.sendto(response, peer)
            print(json.dumps({'time': time.monotonic(), 'response': 'SERVFAIL' if failed else current['ip']}), flush=True)


def health(url):
    """Read only the fixture's public health endpoint with a bounded socket wait."""
    try:
        with urllib.request.urlopen(url, timeout=1) as response:
            return response.status == 200 and json.load(response) == {'status': 'ok'}
    except (OSError, ValueError):
        return False


def wait_health(url, seconds):
    """Return elapsed recovery time without relaxing the caller's deadline."""
    started = time.monotonic()
    harness().wait_until(lambda: health(url), f'health remained unavailable for {seconds}s', seconds, 0.2)
    return time.monotonic() - started


def cleanup_fixture(project, names, failing):
    """Bound every cleanup attempt and preserve an existing scenario failure."""
    integration = harness()
    commands = [('diagnostic', ('logs', '--tail', '200', name)) for name in names[1:3]]
    commands += [('remove', ('rm', '-f', name)) for name in names]
    commands.append(('remove', ('network', 'rm', project)))
    commands += [('inventory', integration.inventory_command(project, resource))
                 for resource in integration.INVENTORY_RESOURCES]
    failures = []
    for phase, args in commands:
        try:
            result = integration.docker(*args, timeout=CLEANUP_TIMEOUT)
        except (subprocess.SubprocessError, OSError) as error:
            print(f'QA {phase} unavailable: {args[0]} {type(error).__name__}', file=sys.stderr)
            if phase != 'diagnostic':
                failures.append((phase, args[0], type(error).__name__))
            continue
        if phase == 'diagnostic':
            lines = (result.stdout + result.stderr).decode(errors='replace').splitlines()
            selected = [line for line in lines[:-10] if 'could not be resolved' in line] + lines[-10:]
            print(f'DIAGNOSTIC {args[-1]}:\n' + '\n'.join(selected), flush=True)
        elif phase == 'inventory' and result.stdout.strip():
            failures.append((args[0], result.stdout.decode().strip()))
    if failures:
        print(f'QA cleanup failed: {failures}', file=sys.stderr)
        if not failing:
            raise AssertionError(f'QA cleanup failed: {failures}')
    else:
        print(f'PASS: {project} resources removed', flush=True)


def run_fixture(args):
    """Replace a real app IP and require unchanged nginx to survive one SERVFAIL."""
    project = 'frayd-dns-' + secrets.token_hex(4)
    label = 'com.docker.compose.project=' + project
    names = [project + suffix for suffix in ('-app', '-dns', '-proxy', '-reserve')]
    app, dns, proxy, reserve = names
    print(f'QA project: {project}', flush=True)
    with tempfile.TemporaryDirectory(prefix=project) as directory:
        folder = Path(directory)
        folder.chmod(0o755)
        try:
            docker('network', 'create', '--label', label, project)

            def start_app():
                """Start only the disposable app with writable ephemeral storage."""
                docker('run', '-d', '--name', app, '--label', label, '--network', project,
                       '--read-only', '--tmpfs', '/tmp', '--tmpfs', '/data:uid=10001,gid=10001', args.app_image)
                return docker('inspect', '--format', '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}', app)

            old_ip = start_app()
            state = folder / 'state.json'
            write_control(state, {'ip': old_ip})
            docker('run', '-d', '--name', dns, '--label', label, '--network', project,
                   '-v', f'{folder}:/control', '-v', f'{Path(__file__).resolve()}:/fixture/test_dns_recovery.py:ro',
                   args.dns_image, 'python', '/fixture/test_dns_recovery.py', '--serve', '/control/state.json')
            harness().wait_until(lambda: 'DNS ready' in docker('logs', dns), 'DNS fixture did not start', 10)
            dns_ip = docker('inspect', '--format', '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}', dns)
            config = Path(args.config).read_text().replace('resolver 127.0.0.11 ', f'resolver {dns_ip} ')
            assert config != Path(args.config).read_text(), 'fixture must override only the resolver address'
            (folder / 'nginx.conf').write_text(config)
            docker('run', '-d', '--name', proxy, '--label', label, '--network', project,
                   '--read-only', '--tmpfs', '/tmp', '-p', '127.0.0.1::8080',
                   '-v', f'{folder}/nginx.conf:/etc/nginx/nginx.conf:ro', args.proxy_image)
            endpoint = 'http://' + docker('port', proxy, '8080/tcp') + '/api/health'
            wait_health(endpoint, 20)
            before = docker('inspect', '--format', '{{.Id}} {{.State.StartedAt}}', proxy)
            docker('rm', '-f', app)
            docker('run', '-d', '--name', reserve, '--label', label, '--network', project,
                   '--entrypoint', 'sleep', args.proxy_image, '120')
            new_ip = start_app()
            assert new_ip != old_ip, (old_ip, new_ip)
            direct = harness().wait_until(
                lambda: docker('exec', proxy, 'wget', '-T', '1', '-qO-', f'http://{new_ip}:8484/api/health'),
                'replacement app never became directly healthy', 10, transient=(subprocess.CalledProcessError,))
            assert json.loads(direct) == {'status': 'ok'}
            write_control(state, {'ip': new_ip, 'fail_next': True})
            harness().wait_until(lambda: not json.loads(state.read_text()).get('fail_next'),
                                 'nginx never refreshed DNS', 10)
            recovered = wait_health(endpoint, 20)
            assert before == docker('inspect', '--format', '{{.Id}} {{.State.StartedAt}}', proxy)
            digest = hashlib.sha256(Path(args.config).read_bytes()).hexdigest()
            print(f'PASS: app IP {old_ip} -> {new_ip}; unchanged proxy recovered after one SERVFAIL in {recovered:.2f}s; '
                  f'config sha256 {digest}', flush=True)
        finally:
            cleanup_fixture(project, names, sys.exc_info()[0] is not None)


def main():
    """Select the pinned DNS fixture server or the real proxy regression command."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--serve')
    parser.add_argument('--app-image', default='frayd-qa:local')
    parser.add_argument('--proxy-image', default='frayd-proxy-qa:local')
    parser.add_argument('--dns-image', default=PYTHON_IMAGE)
    parser.add_argument('--config', default=str(ROOT / 'nginx.conf'))
    args = parser.parse_args()
    if args.serve:
        serve_dns(args.serve)
    else:
        run_fixture(args)


if __name__ == '__main__':
    main()
