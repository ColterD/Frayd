"""Failure-boundary regressions for the disposable Docker integration runner."""
import contextlib
import ast
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


class ThumbnailOracleTests(unittest.TestCase):
    """Reject false artwork ownership before the destructive HTTP oracle runs."""

    def test_exact_database_selected_uuid_jpeg_and_unrelated_siblings(self):
        """Capture only the selected regular JPEG while retaining unrelated artwork."""
        with tempfile.TemporaryDirectory() as directory:
            data = Path(directory)
            thumbs = data / 'thumbs'
            thumbs.mkdir()
            name = '7-12345678-1234-4234-8234-123456789abc.jpg'
            selected = thumbs / name
            jpeg = b'\xff\xd8\xffowned JPEG fixture\xff\xd9'
            selected.write_bytes(jpeg)
            legacy = thumbs / '7.jpg'
            legacy.write_bytes(b'legacy retained')
            unknown = thumbs / 'unknown.jpg'
            unknown.write_bytes(b'unknown retained')
            self.assertEqual(integration.qualified_thumbnail(data, 7, '/data/thumbs/' + name),
                             (selected, jpeg))
            self.assertEqual(legacy.read_bytes(), b'legacy retained')
            self.assertEqual(unknown.read_bytes(), b'unknown retained')

    def test_invalid_database_paths_and_artifacts_cannot_pass(self):
        """Missing bytes, aliases, wrong ownership and non-JPEG bytes fail closed."""
        with tempfile.TemporaryDirectory() as directory:
            data = Path(directory)
            thumbs = data / 'thumbs'
            thumbs.mkdir()
            name = '7-12345678-1234-4234-8234-123456789abc.jpg'
            saved = '/data/thumbs/' + name
            selected = thumbs / name
            selected.write_bytes(b'\xff\xd8\xffowned JPEG fixture\xff\xd9')
            invalid = [None, '', '/data/thumbs/7.jpg', saved.replace('7-', '8-', 1),
                       saved.upper(), saved.replace('789abc', '789ABC'), saved.replace('789abc', '789abz'),
                       saved.replace('/thumbs/', '/other/'), saved.replace('/thumbs/', '/thumbs/../thumbs/'),
                       saved.replace('/thumbs/', '//thumbs/'), saved + '.jpg',
                       saved.replace('12345678-1234-4234-8234-', '12345678123442348234')]
            for path in invalid:
                with self.subTest(path=path), self.assertRaises((AssertionError, ValueError)):
                    integration.qualified_thumbnail(data, 7, path)
            for content in (b'', b'PNG bytes', b'\xff\xd8\xfftruncated', b'wrong header\xff\xd9'):
                selected.write_bytes(content)
                with self.subTest(content=content), self.assertRaises(AssertionError):
                    integration.qualified_thumbnail(data, 7, saved)
            selected.unlink()
            with self.assertRaises(AssertionError):
                integration.qualified_thumbnail(data, 7, saved)
            outside = data / 'outside.jpg'
            outside.write_bytes(b'\xff\xd8\xffoutside JPEG fixture\xff\xd9')
            selected.symlink_to(outside)
            with self.assertRaises(AssertionError):
                integration.qualified_thumbnail(data, 7, saved)
            selected.unlink()
            selected.mkdir()
            with self.assertRaises(AssertionError):
                integration.qualified_thumbnail(data, 7, saved)
            selected.rmdir()
            thumbs.rmdir()
            thumbs.symlink_to(data, target_is_directory=True)
            with self.assertRaises(AssertionError):
                integration.qualified_thumbnail(data, 7, saved)


class CurlHttpTests(unittest.TestCase):
    """Execute the credential-safe curl -i adapter against an owned HTTP server."""

    def test_real_http_status_body_and_in_memory_authorization(self):
        """Prove curl returns a conflict body while keeping the bearer capability out of logs."""
        observed = []
        capability = 'ephemeral-fixture-only'

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                """Capture the submitted path, bearer header and body before returning HTTP 409."""
                observed.append((self.path, self.headers.get('Authorization'),
                                 self.rfile.read(int(self.headers['Content-Length']))))
                self.send_response(409)
                self.end_headers()
                self.wfile.write(b'{"reason":"busy"}')

            def log_message(self, *args):
                """Suppress fixture access logs that could expose the test capability."""
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
        """Reject unauthenticated host upgrade and validate the authenticated handshake bytes."""
        token = secrets.token_hex(32)
        proof = secrets.token_hex(16)
        observations = []

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def do_GET(self):
                """Accept only the creator bearer identity and return the matching WebSocket proof."""
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
                """Suppress fixture access logs containing identity and protocol details."""
                pass

        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        output = io.StringIO()

        def create_party(path, data, supplied_token):
            """Supply a party code only when the caller uses the in-memory creator capability."""
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
        """Assign finite budgets to supported Docker verbs and reject unknown operations."""
        cases = [
            (('inspect', 'owned'), ('metadata', 20)),
            (('build', '-t', 'owned-probe', '/owned-context'), ('context-build', 600)),
            (('create', 'owned-probe', '/unused'), ('container-create', 30)),
            (('export', 'owned-container'), ('container-export', 60)),
            (('rm', '-f', 'owned-container'), ('cleanup', 60)),
            (('image', 'rm', '-f', 'owned-probe'), ('cleanup', 60)),
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
        for arguments in [(), ('unknown',), ('image', 'prune'), ('compose', '-f', '-', 'unknown')]:
            with self.assertRaises(ValueError):
                integration.docker_timeout(arguments)

    def test_context_probe_shared_helper_operations_have_budgets(self):
        """Cover every Docker call made by the build-context probe with an operation budget."""
        # Inspect the actual importing caller, so an added operation cannot silently
        # escape coverage by changing only this test's representative argv list.
        source = Path(__file__).parents[1] / 'scripts/check-build-context.py'
        calls = [node for node in ast.walk(ast.parse(source.read_text()))
                 if isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == 'docker']
        self.assertEqual(len(calls), 5)
        for call in calls:
            args = tuple(node.value if isinstance(node, ast.Constant) else 'owned-fixture' for node in call.args)
            with self.subTest(operation=args[:2]):
                operation, timeout = integration.docker_timeout(args)
                self.assertIn(operation, ('context-build', 'container-create', 'container-export', 'cleanup'))
                self.assertGreater(timeout, 0)

    def test_compose_exec_build_info_reaches_bounded_cli_with_stdin_config(self):
        """Dispatch the exact provenance command through a real disposable CLI adapter."""
        arguments = ('compose', '-p', 'owned', '-f', '-', 'exec', '-T', 'app', 'frayd-server', '--build-info')
        with tempfile.TemporaryDirectory() as folder:
            executable = Path(folder) / 'docker'
            executable.write_text(f'#!{sys.executable}\nimport json,sys\n'
                                  f'assert sys.argv[1:] == {list(arguments)!r}\n'
                                  'assert json.load(sys.stdin) == {"services": {"app": {}}}\n'
                                  'print(json.dumps({"version":"0.1.1","commit":"fixture"}))\n')
            executable.chmod(0o755)
            with patch.dict(os.environ, {'PATH': folder}):
                result = integration.docker(*arguments, input=b'{"services":{"app":{}}}')
            self.assertEqual(result.returncode, 0)
            self.assertEqual(json.loads(result.stdout), {'version': '0.1.1', 'commit': 'fixture'})
            self.assertEqual(integration.docker_timeout(arguments), ('compose-exec', 30))

    def test_timeout_override_is_finite_and_diagnostics_are_public(self):
        """Reject zero, negative and non-finite Docker timeout overrides."""
        for timeout in [0, -1, float('inf'), float('nan')]:
            with self.subTest(timeout=timeout), self.assertRaises(ValueError):
                integration.docker('inspect', 'owned', timeout=timeout)

    def test_actual_timeout_stops_child_after_cli_parent_exits(self):
        """Kill a Compose descendant holding pipes after its CLI parent has exited."""
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
            if isinstance(error, urllib.error.HTTPError):
                self.addCleanup(error.close)
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
            if isinstance(error, urllib.error.HTTPError):
                self.addCleanup(error.close)
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


class ScanStatusPollingTests(unittest.TestCase):
    """Keep scan-status transport retries inside the finite idle wait."""

    def test_transport_failure_recovers_without_replaying_scan_post(self):
        """A failed status GET can recover; the scan admission POST runs once."""
        paths = []

        def response(path, *args, **kwargs):
            paths.append(path)
            if path == '/api/scan':
                return b''
            if paths.count('/api/scan/status') == 1:
                raise urllib.error.URLError('temporary disconnect')
            return b'{"scanning":false}'

        with (patch.object(integration, 'request', side_effect=response),
              patch.object(integration.time, 'sleep')):
            integration.request('/api/scan', {}, expected=202)
            integration.wait_for_scan_idle()
        self.assertEqual(paths, ['/api/scan', '/api/scan/status', '/api/scan/status'])

    def test_persistent_transport_failure_stops_at_deadline(self):
        """The last transport failure is named after a finite number of polls."""
        with (patch.object(integration, 'request', side_effect=TimeoutError('timed out')) as request,
              patch.object(integration.time, 'monotonic', side_effect=[0, 0, 29.25, 29.5, 29.75, 30]),
              patch.object(integration.time, 'sleep') as sleep,
              self.assertRaisesRegex(AssertionError, 'scan.*TimeoutError')):
            integration.wait_for_scan_idle()
        self.assertEqual(request.call_count, 2)
        self.assertEqual([call.args for call in request.call_args_list],
                         [('/api/scan/status',), ('/api/scan/status',)])
        self.assertEqual([call.kwargs['timeout'] for call in request.call_args_list], [2, 0.5])
        self.assertEqual([call.args[0] for call in sleep.call_args_list], [0.5, 0.25])

    def test_scan_post_transport_failure_is_not_replayed(self):
        """The request adapter leaves a failed mutation to its caller."""
        with (patch.object(integration, 'base', 'http://127.0.0.1:1'),
              patch.object(integration.urllib.request, 'urlopen', side_effect=TimeoutError('timed out')) as open_url,
              self.assertRaises(TimeoutError)):
            integration.request('/api/scan', {}, expected=202)
        open_url.assert_called_once()

    def test_http_denial_and_invalid_json_fail_fast(self):
        """Unexpected status and invalid payload are application failures, not retryable transport."""
        for failure in (AssertionError('status 403'),
                        urllib.error.HTTPError('/api/scan/status', 403, 'denied', None, None)):
            with (self.subTest(failure=type(failure).__name__),
                  patch.object(integration, 'request', side_effect=failure) as request,
                  patch.object(integration.time, 'sleep') as sleep,
                  self.assertRaises(type(failure))):
                integration.wait_for_scan_idle()
            request.assert_called_once()
            sleep.assert_not_called()
            if isinstance(failure, urllib.error.HTTPError):
                failure.close()
        with (patch.object(integration, 'request', return_value=b'not JSON') as request,
              patch.object(integration.time, 'sleep') as sleep,
              self.assertRaises(json.JSONDecodeError)):
            integration.wait_for_scan_idle()
        request.assert_called_once()
        sleep.assert_not_called()


if __name__ == '__main__':
    unittest.main()
