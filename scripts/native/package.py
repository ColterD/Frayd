#!/usr/bin/env python3
"""Package a native bundle archive. Build-time only; installed applications do not need Python."""
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import stat
import struct
import subprocess
import sys
import tarfile

SOURCE = Path(__file__).resolve().parents[2]
TARGETS = {'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu', 'aarch64-apple-darwin', 'x86_64-apple-darwin'}
SCRIPTS = {'install.sh': 'scripts/native/install.sh', 'start.sh': 'scripts/native/start.sh', 'uninstall.sh': 'scripts/uninstall.sh'}
EXECUTABLES = {'frayd-server', 'frayd-native', *SCRIPTS}
README = '''Frayd experimental native bundle

Extract with preserved manifest modes (including under umask 077):
  tar -xpzf frayd-<version>-<target>.tar.gz
Install (choose NEW absolute private prefix/data paths with existing parents):
  ./install.sh --prefix /absolute/new/frayd --data-dir /absolute/new/frayd-data
Start in foreground:
  ./start.sh --prefix /absolute/new/frayd --port 8484
Remove only unchanged owned program files, preserving ALL data:
  ./uninstall.sh --prefix /absolute/new/frayd

Add --yes to install/uninstall only for unattended confirmation. No upgrades,
services, automatic browser opening, library deletion or toolchain management.
Configure library roots in the application. HTTP binds to loopback. External
ffmpeg/ffprobe are required for media features; they are not bundled or qualified
by the basic native lifecycle suite. Keep the extracted helper for removal retry.
Normal foreground Ctrl-C reaps only the owned server. A supervisor crash or
SIGKILL leaves an orphan receipt holding start/removal for manual inspection;
no unrelated process is guessed or killed. All data survives uninstall.
manifest.json lists every packaged file's size, mode and SHA-256 and records the
packaging checkout's commit. build-info.json is copied from the server binary and
says whether that build was dirty. macOS archives are unsigned.
'''


def check_binary(binary, target):
    path = Path(binary)
    if not path.is_absolute() or not path.is_file() or path.is_symlink():
        raise SystemExit('Binary must be an absolute regular file')
    with path.open('rb') as stream:
        header = stream.read(64)
    if target.endswith('apple-darwin'):
        expected_cpu = 0x0100000c if target.startswith('aarch64') else 0x01000007
        valid = len(header) >= 8 and struct.unpack('<II', header[:8]) == (0xfeedfacf, expected_cpu)
    else:
        expected_cpu = 183 if target.startswith('aarch64') else 62
        valid = len(header) >= 20 and header[:6] == b'\x7fELF\x02\x01' and struct.unpack('<H', header[18:20])[0] == expected_cpu
    if not valid:
        raise SystemExit('Binary format/architecture does not match native target')


def validate_helper_identity(binary, server_identity):
    try:
        identity = json.loads(subprocess.check_output([binary, '--build-info'], text=True, timeout=10))
    except (OSError, subprocess.SubprocessError, ValueError) as error:
        raise SystemExit(f'Helper binary build identity unavailable: {error}') from error
    if not isinstance(identity, dict) or identity.get('role') != 'frayd-native':
        raise SystemExit('Helper binary must report role frayd-native')
    if (not isinstance(identity.get('version'), str)
            or not isinstance(identity.get('commit'), str)
            or not re.fullmatch('[0-9a-f]{40}', identity['commit'])
            or 'dirty' not in identity
            or (identity['dirty'] is not None and type(identity['dirty']) is not bool)
            or 'commits_since_tag' not in identity
            or (identity['commits_since_tag'] is not None
                and (type(identity['commits_since_tag']) is not int or identity['commits_since_tag'] < 0))):
        raise SystemExit('Helper binary build identity is malformed or unknown')
    if any(identity[key] != server_identity.get(key)
           for key in ('version', 'commit', 'dirty', 'commits_since_tag')):
        raise SystemExit('Helper binary build identity must match server version, commit, dirty flag and tag distance')


def git(*arguments):
    return subprocess.check_output(['git', '-C', str(SOURCE), *arguments], text=True)


def toolchain_metadata():
    path = os.environ.get('FRAYD_NATIVE_TOOLCHAIN_FILE')
    if path:
        toolchain_file = Path(path)
        if not toolchain_file.is_absolute() or toolchain_file.is_symlink() or not toolchain_file.is_file():
            raise SystemExit('Toolchain metadata must be an absolute regular file')
        toolchain = toolchain_file.read_text().strip()
    else:
        toolchain = subprocess.check_output(['rustc', '+1.99.1', '--version', '--verbose'], text=True).strip()
    lines = toolchain.splitlines()
    if not toolchain.startswith('rustc ') or not any(line.startswith('host: ') for line in lines) or not any(line.startswith('release: ') for line in lines):
        raise SystemExit('Complete actual rustc --version --verbose metadata is required')
    return toolchain


def main():
    if len(sys.argv) != 5:
        print('Usage: package.py ABSOLUTE_NEW_OUTPUT TARGET ABSOLUTE_SERVER ABSOLUTE_HELPER', file=sys.stderr)
        return 2
    output, target, server, helper = sys.argv[1:]
    output = Path(output)
    if not output.is_absolute() or output.exists() or output.is_symlink() or not output.parent.is_dir():
        raise SystemExit('Output must be an absolute new directory with an existing parent')
    if target not in TARGETS:
        raise SystemExit('Unsupported native target')
    check_binary(server, target)
    check_binary(helper, target)
    # Reject same-path, hardlinked and copied servers before executing either input.
    # Role metadata detects accidental substitutions; it is not binary authentication.
    if hashlib.sha256(Path(server).read_bytes()).digest() == hashlib.sha256(Path(helper).read_bytes()).digest():
        raise SystemExit('Server and helper binaries must differ')
    client_paths = [Path(name) for name in git('ls-files', '--', 'client').splitlines()]
    for relative in client_paths:
        if any(part.startswith('.env') for part in relative.parts):
            raise SystemExit('Environment files must never be bundled')
        path = SOURCE / relative
        if path.is_symlink() or not path.is_file() or any(parent.is_symlink() for parent in path.parents):
            raise SystemExit('Client redirects/special files refused')
    source_sha = git('rev-parse', 'HEAD').strip()
    build_info = json.loads(subprocess.check_output([server, '--build-info'], text=True, timeout=10))
    if build_info.get('commit') != source_sha:
        raise SystemExit('Server binary build commit must match packaging checkout; unknown/mismatched binary refused')
    version = next(line.split('"')[1] for line in (SOURCE / 'server/Cargo.toml').read_text().splitlines() if line.startswith('version = '))
    if build_info.get('version') != version:
        raise SystemExit('Server binary package version must match Cargo version')
    validate_helper_identity(helper, build_info)
    toolchain = toolchain_metadata()

    output.mkdir(mode=0o700)
    bundle = output / f'frayd-{version}-{target}'
    bundle.mkdir(mode=0o700)
    shutil.copy2(server, bundle / 'frayd-server')
    shutil.copy2(helper, bundle / 'frayd-native')
    (bundle / 'build-info.json').write_text(json.dumps(build_info, indent=2) + '\n')
    for relative in client_paths:
        destination = bundle / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(SOURCE / relative, destination)
    for name, relative in SCRIPTS.items():
        shutil.copy2(SOURCE / relative, bundle / name)
    (bundle / 'README.md').write_text(README)
    for path in sorted(bundle.rglob('*')):
        executable = path.is_dir() or (path.parent == bundle and path.name in EXECUTABLES)
        path.chmod(0o755 if executable else 0o644)
    entries = []
    for path in sorted(bundle.rglob('*')):
        is_file = path.is_file()
        entries.append({
            'path': path.relative_to(bundle).as_posix(),
            'kind': 'dir' if path.is_dir() else 'file',
            'mode': stat.S_IMODE(path.stat().st_mode),
            'size': path.stat().st_size if is_file else 0,
            'sha256': hashlib.sha256(path.read_bytes()).hexdigest() if is_file else '',
        })
    manifest = {'schema': 1, 'version': version, 'source': source_sha, 'target': target, 'toolchain': toolchain, 'entries': entries}
    (bundle / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    archive = output / (bundle.name + '.tar.gz')
    with tarfile.open(archive, 'w:gz') as tar:
        tar.add(bundle, arcname=bundle.name)
    archive.with_name(archive.name + '.sha256').write_text(hashlib.sha256(archive.read_bytes()).hexdigest() + '  ' + archive.name + '\n')
    print(archive)
    return 0


if __name__ == '__main__':
    sys.exit(main())
