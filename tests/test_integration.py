"""Failure-boundary regressions for the disposable Docker integration runner."""
import contextlib
import base64
import hashlib
import http.client
import http.server
import importlib.util
import io
import fcntl
import json
import os
import secrets
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import urllib.error
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('integration', Path(__file__).parents[1] / 'scripts/integration.py')
integration = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(integration)


class CurlHttpTests(unittest.TestCase):
    """Execute the credential-safe curl -i adapter against an owned HTTP server."""

    def test_real_http_status_body_and_in_memory_authorization(self):
        observed = []
        capability = 'ephemeral-fixture-only'

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                observed.append((self.path, self.headers.get('Authorization'),
                                 self.rfile.read(int(self.headers['Content-Length']))))
                self.send_response(409)
                self.end_headers()
                self.wfile.write(b'{"reason":"busy"}')

            def log_message(self, *args):
                pass

        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        output = io.StringIO()
        try:
            with patch.object(integration, 'base', f'http://127.0.0.1:{server.server_port}'), contextlib.redirect_stdout(output):
                result = integration.curl_request('/owned-settings', {'library_paths': []},
                                                  capability, 'POST', 409)
            self.assertEqual(result, b'{"reason":"busy"}')
            self.assertEqual(observed, [('/owned-settings', 'Bearer ' + capability,
                                         b'{"library_paths": []}')])
            self.assertNotIn(capability, output.getvalue())
            self.assertIn('HTTP curl-i POST /owned-settings: 409 expected 409', output.getvalue())
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)
        self.assertFalse(thread.is_alive())


class WebSocketIdentityWireTests(unittest.TestCase):
    """Check actual upgrade bytes against an independent creator-account wire oracle."""

    def test_wrong_device_refused_then_creator_account_and_protocol_accept_verified(self):
        token = secrets.token_hex(32)
        proof = secrets.token_hex(16)
        observations = []

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def do_GET(self):
                canonical_account = self.headers.get('Authorization') == 'Bearer ' + token
                proof_matches = self.headers.get('Sec-WebSocket-Protocol') == 'frayd-party, frayd-host.' + proof
                observations.append({'canonicalAccount': canonical_account, 'proofMatches': proof_matches})
                if not canonical_account:
                    self.send_response(403)
                    self.send_header('Content-Length', '0')
                    self.end_headers()
                    return
                accept = base64.b64encode(hashlib.sha1(
                    (self.headers['Sec-WebSocket-Key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode(),
                    usedforsecurity=False).digest()).decode()
                self.send_response(101)
                self.send_header('Upgrade', 'websocket')
                self.send_header('Connection', 'Upgrade')
                self.send_header('Sec-WebSocket-Accept', accept)
                self.send_header('Sec-WebSocket-Protocol', 'frayd-party')
                self.end_headers()
                self.close_connection = True

            def log_message(self, *args):
                pass

        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        output = io.StringIO()

        def create_party(path, data, supplied_token):
            self.assertEqual((path, data), ('/api/party', {'identity': 'qa-host'}))
            self.assertTrue(supplied_token == token, 'Party creation uses the same in-memory account capability')
            return json.dumps({'code': 'ABCD', 'host_capability': proof}).encode()

        try:
            with (patch.object(integration, 'base', f'http://127.0.0.1:{server.server_port}'),
                  patch.object(integration, 'request', side_effect=create_party),
                  contextlib.redirect_stdout(output)):
                integration.websocket_upgrade(token)
            self.assertEqual(observations, [{'canonicalAccount': False, 'proofMatches': True},
                                            {'canonicalAccount': True, 'proofMatches': True}])
            self.assertTrue(token not in output.getvalue() and proof not in output.getvalue(),
                            'Credential and proof stay out of public diagnostics')
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)
        self.assertFalse(thread.is_alive())


class DockerTimeoutTests(unittest.TestCase):
    """Prove control/lifecycle/media budgets and timeout descendant ownership."""

    def test_operation_budgets_and_unknown_operations(self):
        cases = [
            (('inspect', 'owned'), ('metadata', 20)),
            (('container', 'ls', '-a'), ('metadata', 20)),
            (('network', 'inspect', 'owned'), ('metadata', 20)),
            (('volume', 'ls'), ('metadata', 20)),
            (('compose', '-p', 'owned', '-f', '-', 'config'), ('metadata', 20)),
            (('compose', '-p', 'owned', '-f', '-', 'up', '--wait', '--wait-timeout', '90'), ('compose-up', 120)),
            (('compose', '-f', '-', 'restart', 'app'), ('lifecycle', 60)),
            (('compose', '-f', '-', 'down', '--volumes'), ('cleanup', 60)),
            (('exec', '-i', 'owned', 'sh', '-c', 'cat'), ('exec', 30)),
            (('exec', 'owned', 'ffmpeg', '-v', 'error'), ('media-conversion', 120)),
            (('exec', 'owned', '/usr/bin/ffprobe', '-v', 'error'), ('exec', 30)),
        ]
        for arguments, expected in cases:
            with self.subTest(operation=expected[0]):
                self.assertEqual(integration.docker_timeout(arguments), expected)
        for arguments in [(), ('unknown',), ('build', '.'), ('compose', '-f', '-', 'unknown')]:
            with self.assertRaises(ValueError):
                integration.docker_timeout(arguments)

    def test_timeout_override_is_finite_and_diagnostics_are_public(self):
        for timeout in [0, -1, float('inf'), float('nan')]:
            with self.subTest(timeout=timeout), self.assertRaises(ValueError):
                integration.docker('inspect', 'owned', timeout=timeout)

    def test_actual_timeout_stops_child_after_cli_parent_exits(self):
        # Model a Docker CLI that leaves a Compose plugin holding its captured pipes.
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            lease = root / 'lease'
            ready = root / 'ready'
            group = root / 'group'
            child = ('import fcntl,pathlib,sys,time; '
                     'handle=open(sys.argv[1], "w"); fcntl.flock(handle,fcntl.LOCK_EX); '
                     'pathlib.Path(sys.argv[2]).write_text("ready"); time.sleep(60)')
            executable = root / 'docker'
            executable.write_text(f'#!{sys.executable}\nimport os,pathlib,subprocess,sys\n'
                                  f'pathlib.Path({str(group)!r}).write_text(str(os.getpgrp()))\n'
                                  f'subprocess.Popen([sys.executable,"-c",{child!r},{str(lease)!r},{str(ready)!r}])\n'
                                  'print("private stdout"); print("private stderr",file=sys.stderr)\n')
            executable.chmod(0o755)
            output = io.StringIO()
            started = time.monotonic()
            try:
                with (patch.dict(os.environ, {'PATH': folder}), contextlib.redirect_stderr(output),
                      self.assertRaises(subprocess.TimeoutExpired) as caught):
                    integration.docker('inspect', 'private-argv', input=b'private stdin', timeout=1)
                self.assertLess(time.monotonic() - started, 4)
                self.assertTrue(ready.exists(), 'Owned child actually started and held its lease')
                with lease.open('a') as handle:
                    fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
                self.assertEqual(caught.exception.cmd, ['docker', 'metadata'])
                self.assertIsNone(caught.exception.output)
                self.assertIsNone(caught.exception.stderr)
                self.assertNotIn('private', str(caught.exception) + output.getvalue())
            finally:
                if group.exists():
                    try:
                        os.killpg(int(group.read_text()), signal.SIGKILL)
                    except ProcessLookupError:
                        pass


class DockerFailureTests(unittest.TestCase):
    """Keep diagnostics useful and preserve the first nonzero failure."""

    def test_docker_reports_bytes_and_text_stderr_without_stdout_or_input(self):
        """Both subprocess modes print stderr and re-raise the original status."""
        for stderr in (b'build failed\n', 'build failed\n'):
            with self.subTest(stderr_type=type(stderr).__name__):
                error = subprocess.CalledProcessError(17, ['docker', 'build'], output=b'private output', stderr=stderr)
                output = io.StringIO()
                with (
                    patch.object(integration, 'run_docker', side_effect=error),
                    contextlib.redirect_stderr(output),
                    self.assertRaises(subprocess.CalledProcessError) as caught,
                ):
                    integration.docker('inspect', 'owned', input=b'private input')
                self.assertIs(caught.exception, error)
                self.assertEqual(caught.exception.returncode, 17)
                self.assertEqual(output.getvalue(), 'build failed\n')

    def test_real_subprocess_exposes_diagnostic_and_nonzero_status(self):
        """A fake Docker executable proves stderr reaches the caller without mocks."""
        with tempfile.TemporaryDirectory() as folder:
            executable = Path(folder) / 'docker'
            executable.write_text(f'#!{sys.executable}\nimport sys\n'
                                  'sys.stderr.write("synthetic Docker failure\\n")\n'
                                  'sys.stdout.write("private stdout")\nsys.exit(23)\n')
            executable.chmod(0o755)
            output = io.StringIO()
            with (
                patch.dict(os.environ, {'PATH': folder}),
                contextlib.redirect_stderr(output),
                self.assertRaises(subprocess.CalledProcessError) as caught,
            ):
                integration.docker('inspect', 'owned', input=b'private stdin')
            self.assertEqual(caught.exception.returncode, 23)
            self.assertEqual(output.getvalue(), 'synthetic Docker failure\n')

    def test_cleanup_preserves_original_failure(self):
        """A cleanup failure cannot replace the original nonzero Docker status."""
        original = subprocess.CalledProcessError(17, ['docker', 'build'])
        failures = (subprocess.CalledProcessError(19, ['docker', 'compose', 'down']),
                    OSError('synthetic Docker unavailable'),
                    subprocess.TimeoutExpired(['docker', 'cleanup'], 60))
        for cleanup in failures:
            with (
                self.subTest(cleanup_type=type(cleanup).__name__),
                contextlib.redirect_stderr(io.StringIO()),
                patch.object(integration, 'docker', side_effect=cleanup),
                self.assertRaises(subprocess.CalledProcessError) as caught,
                integration.cleanup_project(integration.docker),
            ):
                raise original
            self.assertIs(caught.exception, original)

    def test_cleanup_failure_is_fatal_after_success(self):
        """Resource cleanup is required even when every HTTP assertion passes."""
        cleanup = subprocess.CalledProcessError(19, ['docker', 'compose', 'down'])
        with (
            patch.object(integration, 'docker', side_effect=cleanup) as compose,
            self.assertRaises(subprocess.CalledProcessError) as caught,
            integration.cleanup_project(compose),
        ):
            pass
        compose.assert_called_once_with('down', '--volumes', '--remove-orphans')
        self.assertIs(caught.exception, cleanup)


class CleanupInventoryTests(unittest.TestCase):
    """Exercise the Docker listing contract for stopped cleanup leftovers."""

    def test_stopped_labelled_container_prevents_cleanup_success(self):
        """Stopped containers appear only when Docker is asked for all states."""
        def list_resources(*args):
            """Model Docker's default running-only view of one stopped container."""
            output = b'stopped-fixture' if args[0] == 'container' and '-a' in args else b''
            return subprocess.CompletedProcess(args, 0, stdout=output)

        with (
            patch.object(integration, 'docker', side_effect=list_resources),
            self.assertRaises(AssertionError) as caught,
        ):
            integration.assert_project_removed('frayd-cleanup-fixture')
        self.assertEqual(caught.exception.args[0],
                         ('container', 'frayd-cleanup-fixture', b'stopped-fixture'))


class ReadinessTests(unittest.TestCase):
    """Keep transient network recovery bounded without hiding terminal failures."""

    @staticmethod
    def transient_errors():
        """Build raw and urllib-wrapped errors emitted by the HTTP request stack."""
        return (OSError('socket reset'), ConnectionResetError('peer reset'),
                TimeoutError('socket timed out'), http.client.RemoteDisconnected('peer disconnected'),
                urllib.error.URLError('connection refused'),
                urllib.error.HTTPError('/api/health', 503, 'unavailable', None, None),
                AssertionError('health status was 503 instead of 200'))

    def test_transient_failure_retries_then_accepts_success(self):
        """Every supported transport/status failure can recover on the next probe."""
        for error in self.transient_errors():
            with (
                self.subTest(error_type=type(error).__name__),
                patch.object(integration, 'request', side_effect=[error, b'{"status":"ok"}']) as request,
                patch.object(integration.time, 'monotonic', side_effect=[0, 0, 0.5]),
                patch.object(integration.time, 'sleep'),
            ):
                integration.ready()
                self.assertEqual(request.call_count, 2)

    def test_deadline_exhaustion_reports_the_last_failure(self):
        """A persistent failure terminates at the 20-second deadline with its cause."""
        for error in self.transient_errors():
            with self.subTest(error_type=type(error).__name__):
                with (
                    patch.object(integration, 'request', side_effect=error) as request,
                    patch.object(integration.time, 'monotonic', side_effect=[10, 10, 29.5, 30]),
                    patch.object(integration.time, 'sleep'),
                    self.assertRaises(AssertionError) as caught,
                ):
                    integration.ready()
                self.assertEqual(str(caught.exception), f'QA server did not become ready: {error}')
                self.assertEqual(request.call_count, 2)

    def test_dns_diagnostics_preserve_original_readiness_failure(self):
        """Unavailable diagnostic commands cannot replace a proxy readiness timeout."""
        original = AssertionError('persistent proxy502')
        output = io.StringIO()
        with (
            patch.object(integration, 'ready', side_effect=original),
            patch.object(integration, 'docker', side_effect=OSError('unavailable')) as docker,
            contextlib.redirect_stderr(output),
            self.assertRaises(AssertionError) as caught,
        ):
            integration.dns_recovery_ready('qa-proxy', 'qa-app')
        self.assertIs(caught.exception, original)
        self.assertEqual(docker.call_count, 5)
        for call in docker.call_args_list:
            self.assertEqual(call.kwargs, {'timeout': 5})
        self.assertIn('DIAGNOSTIC Docker DNS unavailable: OSError', output.getvalue())
        self.assertNotIn('Config', str(docker.call_args_list))


if __name__ == '__main__':
    unittest.main()
