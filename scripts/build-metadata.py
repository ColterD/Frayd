"""Capture compile provenance before isolating sources; no runtime/network access."""
import json
import os
import re
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def git(*args):
    if not (ROOT / '.git').exists():
        return None
    result = subprocess.run(['git', '-C', str(ROOT), *args], capture_output=True, text=True, check=False)
    return result.stdout.strip() if result.returncode == 0 else None


def capture():
    commit = git('rev-parse', 'HEAD')
    tag = git('describe', '--tags', '--abbrev=0')
    complete = git('rev-parse', '--is-shallow-repository') == 'false'
    count = git('rev-list', '--count', f'{tag}..HEAD') if tag and complete else None
    commit = commit if commit and re.fullmatch(r'[0-9a-f]{40}', commit) else None
    count = count if count and count.isascii() and count.isdecimal() else None
    status = git('status', '--porcelain', '--untracked-files=normal')
    return {
        'FRAYD_BUILD_COMMIT': commit or 'unknown',
        'FRAYD_BUILD_COMMITS_SINCE_TAG': count or 'unknown',
        'FRAYD_BUILD_DIRTY': 'unknown' if status is None else str(bool(status)).lower(),
    }


def main():
    metadata = capture()
    if sys.argv[1:] == ['--env']:
        for key, value in metadata.items():
            print(f'{key}={value}')
        return
    if len(sys.argv) > 2 and sys.argv[1] == '--':
        command = sys.argv[2:]
        if command[:2] == ['docker', 'build']:
            command[2:2] = [item for key, value in metadata.items() for item in ('--build-arg', f'{key}={value}')]
        raise SystemExit(subprocess.run(command, env=dict(os.environ, **metadata), check=False).returncode)
    if len(sys.argv) != 1:
        raise SystemExit('Usage: build-metadata.py [--env | -- COMMAND ...]')
    print(json.dumps(metadata, sort_keys=True))


if __name__ == '__main__':
    main()
