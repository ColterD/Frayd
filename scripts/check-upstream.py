"""Operator-only, bounded authenticated Git freshness probe; never changes checkout refs."""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
SHA = re.compile(r'[0-9a-f]{40}')


def git(repo, *args, timeout=45):
    return subprocess.run(['git', '-C', str(repo), *args], capture_output=True, text=True,
                          timeout=timeout, check=False, env=dict(os.environ, GIT_TERMINAL_PROMPT='0', GCM_INTERACTIVE='never'))


def probe(repo, remote='upstream', selected='HEAD', build_url=None):
    result = {'status': 'unknown', 'checked_at': datetime.now(timezone.utc).isoformat(),
              'selection': 'installed build' if build_url else 'checkout ref',
              'selected_commit': None, 'upstream_commit': None, 'upstream_committed_at': None}
    deadline = time.monotonic() + 90
    def run_git(directory, *arguments):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise subprocess.TimeoutExpired('git', 90)
        return git(directory, *arguments, timeout=min(45, remaining))
    try:
        dirty = None
        if build_url:
            url = urllib.parse.urlsplit(build_url)
            if url.scheme not in {'http', 'https'} or url.username or url.password or url.query or url.fragment:
                raise ValueError('invalid build endpoint')
            # Redirects are refused so an operator-selected local endpoint cannot redirect credentials/requests.
            class NoRedirect(urllib.request.HTTPRedirectHandler):
                def redirect_request(self, req, fp, code, msg, headers, newurl):
                    return None
            with urllib.request.build_opener(NoRedirect).open(build_url.rstrip('/') + '/api/build', timeout=10) as response:
                body = bytearray()
                body_deadline = min(deadline, time.monotonic() + 10)
                while True:
                    if time.monotonic() >= body_deadline:
                        raise ValueError('build body deadline exceeded')
                    chunk = response.read1(4097 - len(body))
                    if time.monotonic() >= body_deadline:
                        raise ValueError('build body deadline exceeded')
                    body.extend(chunk)
                    if len(body) > 4096:
                        raise ValueError('build body too large')
                    if not chunk:
                        break
                info = json.loads(body)
            if not isinstance(info, dict):
                raise ValueError('invalid build metadata')
            selected = info.get('commit')
            dirty_value = info.get('dirty')
            dirty = dirty_value if type(dirty_value) is bool else None
            if not isinstance(selected, str) or not SHA.fullmatch(selected):
                raise ValueError('unknown installed commit')
        else:
            resolved = run_git(repo, 'rev-parse', '--verify', '--end-of-options', selected + '^{commit}')
            selected = resolved.stdout.strip() if resolved.returncode == 0 else None
            if not selected or not SHA.fullmatch(selected):
                raise ValueError('unknown selected commit')
        result['selected_commit'] = selected
        result['dirty'] = dirty
        shallow = run_git(repo, 'rev-parse', '--is-shallow-repository')
        if shallow.returncode != 0 or shallow.stdout.strip() != 'false':
            raise ValueError('shallow or unavailable history')
        remotes = run_git(repo, 'remote')
        if remote not in remotes.stdout.splitlines():
            raise ValueError('missing named remote')
        address = run_git(repo, 'remote', 'get-url', remote)
        if address.returncode != 0:
            raise ValueError('remote unavailable')
        remote_address = address.stdout.strip()
        parsed_address = urllib.parse.urlsplit(remote_address)
        if parsed_address.scheme in {'http', 'https'} and (parsed_address.username or parsed_address.password):
            raise ValueError('credential-bearing remote URL refused')
        with tempfile.TemporaryDirectory(prefix='frayd-freshness-') as directory:
            if run_git(directory, 'init', '--bare', '--quiet').returncode != 0:
                raise ValueError('temporary object store unavailable')
            # Full history only in disposable storage; no FETCH_HEAD or refs written to source repo.
            fetched = run_git(directory, 'fetch', '--quiet', '--no-tags', '--', remote_address, 'refs/heads/main:refs/heads/upstream-main')
            if fetched.returncode != 0:
                raise ValueError('upstream fetch failed')
            upstream = run_git(directory, 'rev-parse', 'refs/heads/upstream-main').stdout.strip()
            if not SHA.fullmatch(upstream):
                raise ValueError('invalid upstream commit')
            result['upstream_commit'] = upstream
            result['upstream_committed_at'] = run_git(directory, 'show', '-s', '--format=%cI', upstream).stdout.strip() or None
            if run_git(directory, 'cat-file', '-e', selected + '^{commit}').returncode != 0:
                if run_git(directory, 'fetch', '--quiet', '--no-tags', '--', str(Path(repo).resolve()), selected).returncode != 0:
                    raise ValueError('selected build history unavailable')
            if selected == upstream:
                result['status'] = 'current'
            else:
                behind = run_git(directory, 'merge-base', '--is-ancestor', selected, upstream).returncode
                ahead = run_git(directory, 'merge-base', '--is-ancestor', upstream, selected).returncode
                if behind not in (0, 1) or ahead not in (0, 1):
                    raise ValueError('ancestry unavailable')
                # Ahead is distinct: a custom branch containing upstream is not an upstream release.
                result['status'] = 'behind' if behind == 0 else 'ahead' if ahead == 0 else 'diverged'
            result['reason'] = 'Git ancestry only; dirty/unknown source state does not qualify binary equivalence'
    except (OSError, ValueError, TypeError, subprocess.TimeoutExpired):
        # Never expose Git stderr, credential-bearing remote URLs or HTTP exception details.
        result['reason'] = 'Check unavailable: verify access, selected commit and complete history; no update performed'
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, default=ROOT)
    parser.add_argument('--remote', default='upstream', help='existing authenticated named remote (default: upstream)')
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument('--ref', default='HEAD', help='local ref; default explicitly checks checkout HEAD, not a running binary')
    selection.add_argument('--build-url', help='operator-selected server base URL; read its immutable /api/build commit')
    args = parser.parse_args()
    result = probe(args.repo, args.remote, args.ref, args.build_url)
    print(json.dumps(result, sort_keys=True))
    return 2 if result['status'] == 'unknown' else 0


if __name__ == '__main__':
    raise SystemExit(main())
