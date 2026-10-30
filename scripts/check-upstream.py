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


class CheckFailed(Exception):
    """A probe step failed; the message is a fixed string that is safe to print."""


class NoRedirect(urllib.request.HTTPRedirectHandler):
    """Refuse redirects so an operator-selected local endpoint cannot bounce the request elsewhere."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def git(repo, *args, timeout=45):
    return subprocess.run(['git', '-C', str(repo), *args], capture_output=True, text=True,
                          timeout=timeout, check=False, env=dict(os.environ, GIT_TERMINAL_PROMPT='0', GCM_INTERACTIVE='never'))


def probe(repo, remote='upstream', selected='HEAD', build_url=None):
    result = {'status': 'unknown', 'checked_at': datetime.now(timezone.utc).isoformat(),
              'selection': 'installed build' if build_url else 'checkout ref',
              'selected_commit': None, 'dirty': None, 'upstream_commit': None, 'upstream_committed_at': None}
    deadline = time.monotonic() + 90
    def run_git(directory, *arguments):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise subprocess.TimeoutExpired('git', 90)
        return git(directory, *arguments, timeout=min(45, remaining))
    try:
        if build_url:
            url = urllib.parse.urlsplit(build_url)
            if url.scheme not in {'http', 'https'} or url.username or url.password or url.query or url.fragment:
                raise CheckFailed('invalid build endpoint')
            with urllib.request.build_opener(NoRedirect).open(build_url.rstrip('/') + '/api/build', timeout=10) as response:
                body = bytearray()
                body_deadline = min(deadline, time.monotonic() + 10)
                while True:
                    if time.monotonic() >= body_deadline:
                        raise CheckFailed('build body deadline exceeded')
                    chunk = response.read1(4097 - len(body))
                    if time.monotonic() >= body_deadline:
                        raise CheckFailed('build body deadline exceeded')
                    body.extend(chunk)
                    if len(body) > 4096:
                        raise CheckFailed('build body too large')
                    if not chunk:
                        break
                try:
                    info = json.loads(body)
                except ValueError:
                    raise CheckFailed('invalid build metadata') from None
            if not isinstance(info, dict):
                raise CheckFailed('invalid build metadata')
            selected = info.get('commit')
            if not isinstance(selected, str) or not SHA.fullmatch(selected):
                raise CheckFailed('unknown installed commit')
            dirty = info.get('dirty')
            result['dirty'] = dirty if type(dirty) is bool else None
        else:
            resolved = run_git(repo, 'rev-parse', '--verify', '--end-of-options', selected + '^{commit}')
            selected = resolved.stdout.strip() if resolved.returncode == 0 else None
            if not selected or not SHA.fullmatch(selected):
                raise CheckFailed('unknown selected commit')
        result['selected_commit'] = selected
        shallow = run_git(repo, 'rev-parse', '--is-shallow-repository')
        if shallow.returncode != 0 or shallow.stdout.strip() != 'false':
            raise CheckFailed('shallow or unavailable history')
        remotes = run_git(repo, 'remote')
        if remote not in remotes.stdout.splitlines():
            raise CheckFailed('missing named remote')
        address = run_git(repo, 'remote', 'get-url', remote)
        if address.returncode != 0:
            raise CheckFailed('remote unavailable')
        remote_address = address.stdout.strip()
        parsed_address = urllib.parse.urlsplit(remote_address)
        if parsed_address.scheme in {'http', 'https'} and (parsed_address.username or parsed_address.password):
            raise CheckFailed('credential-bearing remote URL refused')
        with tempfile.TemporaryDirectory(prefix='frayd-freshness-') as directory:
            if run_git(directory, 'init', '--bare', '--quiet').returncode != 0:
                raise CheckFailed('temporary object store unavailable')
            # Full history only in disposable storage; no FETCH_HEAD or refs written to source repo.
            fetched = run_git(directory, 'fetch', '--quiet', '--no-tags', '--', remote_address, 'refs/heads/main:refs/heads/upstream-main')
            if fetched.returncode != 0:
                raise CheckFailed('upstream fetch failed')
            upstream = run_git(directory, 'rev-parse', 'refs/heads/upstream-main').stdout.strip()
            if not SHA.fullmatch(upstream):
                raise CheckFailed('invalid upstream commit')
            result['upstream_commit'] = upstream
            result['upstream_committed_at'] = run_git(directory, 'show', '-s', '--format=%cI', upstream).stdout.strip() or None
            if run_git(directory, 'cat-file', '-e', selected + '^{commit}').returncode != 0:
                if run_git(directory, 'fetch', '--quiet', '--no-tags', '--', str(Path(repo).resolve()), selected).returncode != 0:
                    raise CheckFailed('selected build history unavailable')
            if selected == upstream:
                result['status'] = 'current'
            else:
                behind = run_git(directory, 'merge-base', '--is-ancestor', selected, upstream).returncode
                ahead = run_git(directory, 'merge-base', '--is-ancestor', upstream, selected).returncode
                if behind not in (0, 1) or ahead not in (0, 1):
                    raise CheckFailed('ancestry unavailable')
                # Ahead is distinct: a custom branch containing upstream is not an upstream release.
                result['status'] = 'behind' if behind == 0 else 'ahead' if ahead == 0 else 'diverged'
            result['reason'] = 'Git ancestry only; dirty/unknown source state does not qualify binary equivalence'
    except CheckFailed as error:
        result['reason'] = f'Check unavailable: {error}; no update performed'
    except (subprocess.TimeoutExpired, TimeoutError):
        result['reason'] = 'Check unavailable: deadline exceeded; no update performed'
    except (OSError, ValueError):
        # Never expose Git stderr, credential-bearing remote URLs or HTTP exception details.
        result['reason'] = 'Check unavailable: Git or the build endpoint could not be reached; no update performed'
    except Exception:
        # Any other failure still ends as the unknown result (exit 2); no exception text or URL is printed.
        result['reason'] = 'Check unavailable: internal error; no update performed'
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
