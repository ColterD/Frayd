"""Failure-boundary regressions for the disposable Docker integration runner."""
import base64
import contextlib
import fcntl
import hashlib
import http.client
import http.server
import importlib.util
import io
import json
import math
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


class FakeClock:
    """Deterministic time source: only sleeping advances it, so deadlines are exact."""

    def __init__(self):
        self.now = 0.0

    def monotonic(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds


@contextlib.contextmanager
def fake_clock():
    """Replace the harness's monotonic clock and sleep with a FakeClock."""
    clock = FakeClock()
    with (patch.object(integration.time, 'monotonic', clock.monotonic),
          patch.object(integration.time, 'sleep', clock.sleep)):
        yield clock


class QuietHandler(http.server.BaseHTTPRequestHandler):
    """Fixture handler that never logs requests, which may carry test capabilities."""

    def log_message(self, *args):
        pass


@contextlib.contextmanager
def serve(handler):
    """Run an owned loopback HTTP server and yield its base URL."""
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f'http://127.0.0.1:{server.server_port}'
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)
        assert not thread.is_alive()


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


class RequestAdapterTests(unittest.TestCase):
    """Execute the HTTP adapter against an owned server."""

    def test_methods_error_bodies_and_in_memory_authorization(self):
        """Return a conflict body, send the bearer capability and print nothing."""
        observed = []
        capability = 'ephemeral-fixture-only'

        class Handler(QuietHandler):
            def respond(self, status, body):
                """Capture the method, path, bearer header and body, then reply."""
                observed.append((self.command, self.path, self.headers.get('Authorization'),
                                 self.rfile.read(int(self.headers.get('Content-Length', 0)))))
                self.send_response(status)
                self.end_headers()
                self.wfile.write(body)

            def do_POST(self):
                self.respond(409, b'{"reason":"busy"}')

            def do_DELETE(self):
                self.respond(204, b'')

        output = io.StringIO()
        with serve(Handler) as url, patch.object(integration, 'base', url), contextlib.redirect_stdout(output):
            conflict = integration.request('/owned-settings', {'library_paths': []}, capability,
                                           expected=409, method='POST')
            integration.request('/owned-user', token=capability, expected=204, method='DELETE')
        self.assertEqual(conflict, b'{"reason":"busy"}')
        bearer = 'Bearer ' + capability
        self.assertEqual(observed, [('POST', '/owned-settings', bearer, b'{"library_paths": []}'),
                                    ('DELETE', '/owned-user', bearer, b'')])
        self.assertEqual(output.getvalue(), '')

    def test_unexpected_status_raises_typed_error_with_headers_and_body(self):
        """Error and success mismatches carry path, status, expectation, headers and body."""
        class Handler(QuietHandler):
            def reply(self, status, headers, body):
                """Send one status with the given headers and body."""
                self.send_response(status)
                for name, value in headers.items():
                    self.send_header(name, value)
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_DELETE(self):
                self.reply(409, {'Retry-After': '1'}, b'held')

            def do_GET(self):
                self.reply(200, {}, b'served')

        with serve(Handler) as url, patch.object(integration, 'base', url):
            for method, status, headers, body in [('DELETE', 409, {'Retry-After': '1'}, b'held'),
                                                  ('GET', 200, {}, b'served')]:
                with self.subTest(method=method):
                    with self.assertRaises(integration.HTTPStatusError) as caught:
                        integration.request('/owned-media', method=method, expected=404)
                    error = caught.exception
                    self.assertIsInstance(error, AssertionError)
                    self.assertEqual((error.path, error.status, error.expected, error.body),
                                     ('/owned-media', status, 404, body))
                    self.assertEqual(error.headers.get('Retry-After'), headers.get('Retry-After'))


class WebSocketIdentityWireTests(unittest.TestCase):
    """Check actual upgrade bytes against an independent creator-account wire oracle."""

    def test_wrong_device_refused_then_creator_account_and_protocol_accept_verified(self):
        """Reject unauthenticated host upgrade and validate the authenticated handshake bytes."""
        token = secrets.token_hex(32)
        proof = secrets.token_hex(16)
        observations = []

        class Handler(QuietHandler):
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

        output = io.StringIO()

        def create_party(path, data, supplied_token):
            """Supply a party code only when the caller uses the in-memory creator capability."""
            self.assertEqual((path, data), ('/api/party', {'identity': 'qa-host'}))
            self.assertTrue(supplied_token == token, 'Party creation uses the same in-memory account capability')
            return json.dumps({'code': 'ABCD', 'host_capability': proof}).encode()

        with (serve(Handler) as url,
              patch.object(integration, 'base', url),
              patch.object(integration, 'request', side_effect=create_party),
              contextlib.redirect_stdout(output)):
            integration.websocket_upgrade(token)
        self.assertEqual(observations, [{'canonicalAccount': False, 'proofMatches': True},
                                        {'canonicalAccount': True, 'proofMatches': True}])
        self.assertTrue(token not in output.getvalue() and proof not in output.getvalue(),
                        'Credential and proof stay out of public diagnostics')


class DockerDeadlineTests(unittest.TestCase):
    """Prove every Docker call is bounded and a timeout stops descendants."""

    def test_default_and_explicit_deadlines_reach_the_runner(self):
        """Calls get the default deadline unless the caller passes its own finite one."""
        with patch.object(integration, 'run_docker',
                          return_value=subprocess.CompletedProcess([], 0)) as run:
            integration.docker('inspect', 'owned')
            integration.docker('build', '-t', 'owned-probe', '/owned-context', timeout=600)
        self.assertEqual([call.args[1] for call in run.call_args_list],
                         [integration.DOCKER_TIMEOUT, 600])
        self.assertTrue(0 < integration.DOCKER_TIMEOUT < math.inf)

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

    def test_timeout_override_must_be_finite_and_positive(self):
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
                self.assertTrue(ready.exists(), 'Descendant started and held its lease before the timeout')
                with lease.open('a') as handle:
                    fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
                self.assertEqual(caught.exception.cmd, ['docker', 'inspect'])
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


BUSY = (409, {'Retry-After': '1'})
REFERENCED = (409, {})


def media_server(replies):
    """Build a handler answering each DELETE with the next (status, headers) reply; record paths."""
    replies = iter(replies)
    paths = []

    class Handler(QuietHandler):
        def do_DELETE(self):
            status, headers = next(replies)
            paths.append(self.path)
            self.send_response(status)
            for name, value in headers.items():
                self.send_header(name, value)
            self.send_header('Content-Length', '0')
            self.end_headers()
    return Handler, paths


class DeleteMediaTests(unittest.TestCase):
    """Media deletion waits out only the 409 that names itself transient with Retry-After."""

    def delete(self, replies, expected):
        """Run the real delete_media() and request() against an owned server under a fake clock."""
        handler, paths = media_server(replies)
        output = io.StringIO()
        with (serve(handler) as url, patch.object(integration, 'base', url), fake_clock() as clock,
              contextlib.redirect_stdout(output)):
            try:
                integration.delete_media(1, 'token', expected)
            finally:
                self.paths, self.now, self.output = paths, clock.now, output.getvalue()

    def test_transient_refusals_are_retried_and_counted(self):
        """Two Retry-After 409s are retried, then the expected status ends the wait."""
        self.delete([BUSY, BUSY, (500, {})], 500)
        self.assertEqual(self.paths, ['/api/media/1'] * 3)
        self.assertEqual(self.output, 'delete media 1: 2 transient 409 retries\n')

    def test_permanent_conflict_fails_without_retrying(self):
        """A 409 without Retry-After is a lasting refusal and fails on the first answer."""
        with self.assertRaises(integration.HTTPStatusError) as caught:
            self.delete([REFERENCED], 204)
        self.assertEqual((caught.exception.status, caught.exception.expected), (409, 204))
        self.assertEqual((len(self.paths), self.now), (1, 0))
        self.assertEqual(self.output, 'delete media 1: 0 transient 409 retries\n')

    def test_lasting_transient_refusal_fails_at_the_deadline(self):
        """Transient refusals stop at the 10-second deadline and report every retry."""
        with self.assertRaises(AssertionError) as caught:
            self.delete([BUSY] * 100, 204)
        self.assertIn('stayed refused', str(caught.exception))
        self.assertGreaterEqual(self.now, 10)
        self.assertLess(self.now, 10.5)
        self.assertEqual(self.output, f'delete media 1: {len(self.paths)} transient 409 retries\n')

    def test_other_mismatch_fails_without_retrying(self):
        """Any non-409 mismatch fails on the first answer."""
        with self.assertRaises(integration.HTTPStatusError) as caught:
            self.delete([(404, {})], 500)
        self.assertEqual(caught.exception.status, 404)
        self.assertEqual((len(self.paths), self.now), (1, 0))


class DeleteConflictTests(unittest.TestCase):
    """Read a refused deletion's Retry-After so the harness can tell busy from referenced."""

    def test_refusal_reports_retry_after_and_other_statuses_fail(self):
        """Busy and referenced 409s differ only in Retry-After; a deletion or other status fails."""
        handler, paths = media_server([BUSY, REFERENCED, (204, {}), (404, {})])
        with serve(handler) as url, patch.object(integration, 'base', url):
            self.assertEqual(integration.conflict_retry_after(7, 'token'), '1')
            self.assertIsNone(integration.conflict_retry_after(7, 'token'))
            with self.assertRaisesRegex(AssertionError, 'media 7 was deleted'):
                integration.conflict_retry_after(7, 'token')
            with self.assertRaises(integration.HTTPStatusError):
                integration.conflict_retry_after(7, 'token')
        self.assertEqual(paths, ['/api/media/7'] * 4)


class WaitUntilTests(unittest.TestCase):
    """Cover the shared polling primitive behind every bounded wait."""

    def test_first_truthy_result_returns_without_sleeping(self):
        """A probe that succeeds immediately never waits."""
        with fake_clock() as clock:
            self.assertEqual(integration.wait_until(lambda: 'ready', 'never', 5), 'ready')
        self.assertEqual(clock.now, 0)

    def test_unlisted_errors_propagate_immediately(self):
        """Only the declared transient errors are retried."""
        with fake_clock() as clock, self.assertRaises(ValueError):
            integration.wait_until(self.fail_with(ValueError('bad payload')), 'never', 5, transient=(OSError,))
        self.assertEqual(clock.now, 0)

    def test_deadline_names_the_last_transient_error(self):
        """A persistent transient failure ends exactly at the deadline with its cause."""
        with fake_clock() as clock, self.assertRaises(AssertionError) as caught:
            integration.wait_until(self.fail_with(OSError('refused')), 'slow', 3, 1, transient=(OSError,))
        self.assertEqual(str(caught.exception), "slow: OSError('refused')")
        self.assertEqual(clock.now, 3)

    def test_clean_falsy_result_forgets_an_earlier_error(self):
        """The failure text reports only the latest observation."""
        outcomes = iter([OSError('refused'), False, False, False])

        def probe():
            outcome = next(outcomes)
            if isinstance(outcome, Exception):
                raise outcome
            return outcome

        with fake_clock(), self.assertRaises(AssertionError) as caught:
            integration.wait_until(probe, 'slow', 3, 1, transient=(OSError,))
        self.assertEqual(str(caught.exception), 'slow')

    @staticmethod
    def fail_with(error):
        """Build a probe that always raises error."""
        def probe():
            raise error
        return probe


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
                fake_clock() as clock,
            ):
                integration.ready()
                self.assertEqual(request.call_count, 2)
                self.assertEqual(clock.now, 0.5)

    def test_deadline_exhaustion_reports_the_last_failure(self):
        """A persistent failure terminates at the 20-second deadline with its cause."""
        for error in self.transient_errors():
            if isinstance(error, urllib.error.HTTPError):
                self.addCleanup(error.close)
            with self.subTest(error_type=type(error).__name__):
                with (
                    patch.object(integration, 'request', side_effect=error) as request,
                    fake_clock() as clock,
                    self.assertRaises(AssertionError) as caught,
                ):
                    integration.ready()
                self.assertEqual(str(caught.exception), f'QA server did not become ready: {error!r}')
                self.assertEqual(clock.now, 20)
                self.assertEqual({call.kwargs['timeout'] for call in request.call_args_list}, {2})

    def test_dns_diagnostics_preserve_original_readiness_failure(self):
        """Unavailable diagnostic commands cannot replace a proxy readiness timeout."""
        original = AssertionError('persistent proxy 502')
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

        with patch.object(integration, 'request', side_effect=response), fake_clock():
            integration.request('/api/scan', {}, expected=202)
            integration.wait_for_scan_idle()
        self.assertEqual(paths, ['/api/scan', '/api/scan/status', '/api/scan/status'])

    def test_persistent_transport_failure_stops_at_deadline(self):
        """The last transport failure is named when the 30-second wait ends."""
        with (patch.object(integration, 'request', side_effect=TimeoutError('timed out')) as request,
              fake_clock() as clock,
              self.assertRaisesRegex(AssertionError, 'scan.*TimeoutError')):
            integration.wait_for_scan_idle()
        self.assertEqual(clock.now, 30)
        self.assertEqual({call.args for call in request.call_args_list}, {('/api/scan/status',)})
        self.assertEqual({call.kwargs['timeout'] for call in request.call_args_list}, {2})

    def test_scan_post_transport_failure_is_not_replayed(self):
        """The request adapter leaves a failed mutation to its caller."""
        with (patch.object(integration, 'base', 'http://127.0.0.1:1'),
              patch.object(integration.urllib.request, 'urlopen', side_effect=TimeoutError('timed out')) as open_url,
              self.assertRaises(TimeoutError)):
            integration.request('/api/scan', {}, expected=202)
        open_url.assert_called_once()

    def test_http_denial_and_invalid_json_fail_fast(self):
        """Unexpected status and invalid payload are application failures, not retryable transport."""
        denied = urllib.error.HTTPError('/api/scan/status', 403, 'denied', None, None)
        self.addCleanup(denied.close)
        with (patch.object(integration, 'base', 'http://127.0.0.1:1'),
              patch.object(integration.urllib.request, 'urlopen', side_effect=denied) as open_url,
              patch.object(integration.time, 'sleep') as sleep,
              self.assertRaises(AssertionError)):
            integration.wait_for_scan_idle()
        open_url.assert_called_once()
        sleep.assert_not_called()
        with (patch.object(integration, 'request', return_value=b'not JSON') as request,
              patch.object(integration.time, 'sleep') as sleep,
              self.assertRaises(json.JSONDecodeError)):
            integration.wait_for_scan_idle()
        request.assert_called_once()
        sleep.assert_not_called()


if __name__ == '__main__':
    unittest.main()
