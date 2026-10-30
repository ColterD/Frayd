"""Freshness oracles use disposable Git histories, never authenticated/live remotes."""
import contextlib
import importlib.util
import io
import itertools
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('freshness', ROOT / 'scripts/check-upstream.py')
freshness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(freshness)


def git(repo, *args):
    return subprocess.check_output(['git', '-C', str(repo), *args], stderr=subprocess.DEVNULL, text=True).strip()


class FakeClock:
    """Deterministic monotonic time that advances only when a test moves it."""

    def __init__(self):
        self.now = 0.0

    def monotonic(self):
        return self.now


class BuildResponse:
    """Context-manager response whose body arrives through read1()."""

    def __init__(self, chunks, on_read=None):
        self.chunks = iter(chunks)
        self.on_read = on_read
        self.reads = 0

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        pass

    def read1(self, _size):
        self.reads += 1
        if self.on_read:
            self.on_read()
        return next(self.chunks, b'')


class Freshness(unittest.TestCase):
    def test_ancestry_and_no_checkout_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory) / 'source'
            repo.mkdir()
            git(repo, 'init', '-b', 'main')
            git(repo, 'config', 'user.name', 'Fixture')
            git(repo, 'config', 'user.email', 'fixture@example.invalid')
            git(repo, 'commit', '--allow-empty', '-m', 'base')
            base = git(repo, 'rev-parse', 'HEAD')
            remote = Path(directory) / 'remote.git'
            git(repo, 'clone', '--bare', str(repo), str(remote))
            git(repo, 'remote', 'add', 'upstream', str(remote))
            self.assertEqual(freshness.probe(repo)['status'], 'current')
            git(repo, 'commit', '--allow-empty', '-m', 'upstream change')
            tip = git(repo, 'rev-parse', 'HEAD')
            git(repo, 'push', 'upstream', 'main')
            git(repo, 'checkout', '--detach', base)
            before = git(repo, 'show-ref')
            result = freshness.probe(repo)
            self.assertEqual(result['status'], 'behind')
            self.assertEqual(result['upstream_commit'], tip)
            self.assertTrue(result['checked_at'])
            self.assertTrue(result['upstream_committed_at'])
            self.assertEqual(git(repo, 'show-ref'), before)
            self.assertEqual(git(repo, 'rev-parse', 'HEAD'), base)
            git(repo, 'commit', '--allow-empty', '-m', 'custom fork')
            self.assertEqual(freshness.probe(repo)['status'], 'diverged')
            git(repo, 'checkout', '--detach', tip)
            git(repo, 'commit', '--allow-empty', '-m', 'ahead')
            self.assertEqual(freshness.probe(repo)['status'], 'ahead')
            missing = freshness.probe(repo, remote='missing')
            self.assertEqual(missing['status'], 'unknown')
            self.assertIn('missing named remote', missing['reason'])
            shallow = Path(directory) / 'shallow'
            git(repo, 'clone', '--depth=1', '--branch', 'main', remote.as_uri(), str(shallow))
            truncated = freshness.probe(shallow, remote='origin')
            self.assertEqual(truncated['status'], 'unknown')
            self.assertIn('shallow or unavailable history', truncated['reason'])
            with patch.object(freshness, 'git', side_effect=subprocess.TimeoutExpired('git', 45)):
                timed_out = freshness.probe(repo)
            self.assertEqual(timed_out['status'], 'unknown')
            self.assertIn('deadline exceeded', timed_out['reason'])

    def test_secret_bearing_remote_is_refused_before_fetch(self):
        outputs = {('rev-parse', '--is-shallow-repository'): 'false', ('remote',): 'upstream',
                   ('remote', 'get-url', 'upstream'): 'https://fixture-user:NONSECRET_FIXTURE@example.invalid/repo'}

        def response(_repo, *args, **_kwargs):
            return subprocess.CompletedProcess([], 0, outputs.get(args, 'a' * 40), '')
        with patch.object(freshness, 'git', side_effect=response) as calls:
            result = freshness.probe(ROOT)
        self.assertEqual(result['status'], 'unknown')
        self.assertIn('credential-bearing remote URL refused', result['reason'])
        for private in ('NONSECRET_FIXTURE', 'fixture-user', 'example.invalid'):
            self.assertNotIn(private, str(result))
        self.assertFalse(any('fetch' in call.args for call in calls.call_args_list))

    def test_dripping_build_body_stops_at_checked_deadline(self):
        clock = FakeClock()

        def drip():
            clock.now += 6

        response = BuildResponse(itertools.repeat(b' '), on_read=drip)
        with patch.object(freshness.urllib.request, 'build_opener') as opener, patch.object(freshness.time, 'monotonic', clock.monotonic):
            opener.return_value.open.return_value = response
            result = freshness.probe(ROOT, build_url='http://127.0.0.1:3100')
        self.assertEqual(result['status'], 'unknown')
        self.assertIn('build body deadline exceeded', result['reason'])
        self.assertEqual(response.reads, 2)

    def test_installed_identity_is_selected_instead_of_checkout(self):
        for payload, reason in [(b'null', 'invalid build metadata'), (b'[]', 'invalid build metadata'),
                                (b'{', 'invalid build metadata'), (b'\xff', 'invalid build metadata'),
                                (b'{"commit":null,"dirty":null}', 'unknown installed commit'),
                                (b'{"commit":false,"dirty":{"unexpected":"NONSECRET_FIXTURE"}}', 'unknown installed commit')]:
            with self.subTest(payload=payload), patch.object(freshness.urllib.request, 'build_opener') as opener:
                opener.return_value.open.return_value = BuildResponse([payload])
                result = freshness.probe(ROOT, build_url='http://127.0.0.1:3100')
            self.assertEqual(result['status'], 'unknown')
            self.assertEqual(result['selection'], 'installed build')
            self.assertIsNone(result['selected_commit'])
            self.assertIn(reason, result['reason'])
            self.assertNotIn('NONSECRET_FIXTURE', str(result))

    def test_unexpected_exception_yields_unknown_without_exception_text(self):
        """Any unexpected exception ends as the JSON unknown result with a fixed reason and exit status 2."""
        secret = 'NONSECRET_FIXTURE https://fixture-user:NONSECRET_PASSWORD@example.invalid/repo'
        for error in (TypeError(secret), KeyError(secret), AttributeError(secret), RuntimeError(secret)):
            with self.subTest(error=type(error).__name__), patch.object(freshness, 'git', side_effect=error):
                result = freshness.probe(ROOT)
            self.assertEqual(result['status'], 'unknown')
            self.assertEqual(result['reason'], 'Check unavailable: internal error; no update performed')
            for private in ('NONSECRET', 'fixture-user', 'example.invalid'):
                self.assertNotIn(private, str(result))
        with patch.object(freshness, 'git', side_effect=TypeError(secret)), patch.object(sys, 'argv', ['check-upstream.py']), \
                contextlib.redirect_stdout(io.StringIO()) as output:
            status = freshness.main()
        self.assertEqual(status, 2)
        self.assertEqual(json.loads(output.getvalue())['status'], 'unknown')
        self.assertNotIn('NONSECRET', output.getvalue())

    def test_unreachable_endpoint_names_no_address(self):
        with patch.object(freshness.urllib.request, 'build_opener') as opener:
            opener.return_value.open.side_effect = OSError('connect to user:NONSECRET_FIXTURE@127.0.0.1 failed')
            result = freshness.probe(ROOT, build_url='http://127.0.0.1:3100')
        self.assertEqual(result['status'], 'unknown')
        self.assertIn('could not be reached', result['reason'])
        self.assertNotIn('NONSECRET_FIXTURE', str(result))
        self.assertNotIn('127.0.0.1', result['reason'])


class LocalBuildMetadata(unittest.TestCase):
    def test_cargo_refreshes_head_dirty_clean_and_refuses_ancestor_git(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'source'
            server = root / 'server'
            (server / 'src').mkdir(parents=True)
            shutil.copyfile(ROOT / 'server/build.rs', server / 'build.rs')
            (server / 'Cargo.toml').write_text('[package]\nname="identity-fixture"\nversion="0.1.1"\nedition="2021"\n')
            (server / 'src/main.rs').write_text('fn main(){ println!("{} {} {}", env!("FRAYD_BUILD_COMMIT"), env!("FRAYD_BUILD_DIRTY"), env!("FRAYD_BUILD_COMMITS_SINCE_TAG")); }')
            git(root, 'init', '-b', 'main')
            git(root, 'config', 'user.name', 'Fixture')
            git(root, 'config', 'user.email', 'fixture@example.invalid')
            (root / '.gitignore').write_text('server/target/\n')
            git(root, 'add', '.')
            git(root, 'commit', '-m', 'fixture')
            environment = {key: value for key, value in os.environ.items() if not key.startswith('FRAYD_BUILD_')}
            def build(path):
                subprocess.run(['cargo', 'build', '--offline', '--quiet', '--manifest-path', str(path / 'Cargo.toml')], env=environment, check=True, capture_output=True, timeout=60)
                return subprocess.check_output([str(path / 'target/debug/identity-fixture')], text=True).strip().split()
            # Cargo.lock is a tracked input before the clean baseline.
            build(server)
            git(root, 'add', 'server/Cargo.lock')
            git(root, 'commit', '-m', 'lock')
            first = build(server)
            self.assertEqual(first, [git(root, 'rev-parse', 'HEAD'), 'false', 'unknown'])
            git(root, 'commit', '--allow-empty', '-m', 'head only')
            self.assertNotEqual(build(server)[0], first[0])
            with (server / 'src/main.rs').open('a') as stream:
                stream.write('\n// changed\n')
            self.assertEqual(build(server)[1], 'true')
            git(root, 'checkout', '--', 'server/src/main.rs')
            self.assertEqual(build(server)[1], 'false')
            # A copy nested inside an unrelated Git worktree must never identify its ancestor.
            copied = root / 'archive/server'
            shutil.copytree(server, copied, ignore=shutil.ignore_patterns('target'))
            self.assertEqual(build(copied), ['unknown', 'unknown', 'unknown'])
            metadata_spec = importlib.util.spec_from_file_location('metadata', ROOT / 'scripts/build-metadata.py')
            metadata = importlib.util.module_from_spec(metadata_spec)
            metadata_spec.loader.exec_module(metadata)
            git(root, 'tag', 'v0.1.1')
            shallow = Path(directory) / 'shallow'
            git(root, 'clone', '--depth=1', root.as_uri(), str(shallow))
            self.assertEqual(build(shallow / 'server')[2], 'unknown')
            with patch.object(metadata, 'ROOT', shallow):
                self.assertEqual(metadata.capture()['FRAYD_BUILD_COMMITS_SINCE_TAG'], 'unknown')
            with patch.object(metadata, 'ROOT', copied.parent):
                self.assertEqual(set(metadata.capture().values()), {'unknown'})
            outputs = {('rev-parse', 'HEAD'): 'bad\nLINE', ('describe', '--tags', '--abbrev=0'): 'v0',
                       ('rev-parse', '--is-shallow-repository'): 'false', ('rev-list', '--count', 'v0..HEAD'): '2\nSECRET',
                       ('status', '--porcelain', '--untracked-files=normal'): ''}
            with patch.object(metadata, 'git', side_effect=lambda *args: outputs[args]):
                captured = metadata.capture()
                self.assertEqual(captured['FRAYD_BUILD_COMMIT'], 'unknown')
                self.assertEqual(captured['FRAYD_BUILD_COMMITS_SINCE_TAG'], 'unknown')
            self.assertEqual(json.loads(subprocess.check_output(['python3', str(ROOT / 'scripts/build-metadata.py')], text=True))['FRAYD_BUILD_COMMIT'], git(ROOT, 'rev-parse', 'HEAD'))
