#!/usr/bin/env bash
# Build glue only. No Python interpreter is required by an installed application.
set -euo pipefail
if [[ $# != 4 ]]; then
  printf '%s\n' 'Usage: package.sh ABSOLUTE_NEW_OUTPUT TARGET ABSOLUTE_SERVER ABSOLUTE_HELPER' >&2
  exit 2
fi
FRAYD_SOURCE=$(cd -- "$(dirname -- "$0")/../.." && pwd -P)
python3 - "$FRAYD_SOURCE" "$@" <<'PY'
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import struct
import subprocess
import sys
import tarfile

source = Path(sys.argv[1])
output, target, server, helper = sys.argv[2:]
output = Path(output)
if not output.is_absolute() or output.exists() or output.is_symlink() or not output.parent.is_dir():
    raise SystemExit('Output must be an absolute new directory with an existing parent')
if target not in {'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu', 'aarch64-apple-darwin', 'x86_64-apple-darwin'}:
    raise SystemExit('Unsupported native target')
for binary in (server, helper):
    if not Path(binary).is_absolute() or not Path(binary).is_file() or Path(binary).is_symlink():
        raise SystemExit('Binary must be an absolute regular file')
    with Path(binary).open('rb') as stream:
        header = stream.read(64)
    if target.endswith('apple-darwin'):
        expected_cpu = 0x0100000c if target.startswith('aarch64') else 0x01000007
        valid = len(header) >= 8 and struct.unpack('<II', header[:8]) == (0xfeedfacf, expected_cpu)
    else:
        expected_cpu = 183 if target.startswith('aarch64') else 62
        valid = len(header) >= 20 and header[:6] == b'\x7fELF\x02\x01' and struct.unpack('<H', header[18:20])[0] == expected_cpu
    if not valid:
        raise SystemExit('Binary format/architecture does not match native target')
client_paths = [Path(name) for name in subprocess.check_output(['git', '-C', str(source), 'ls-files', '--', 'client'], text=True).splitlines()]
for relative in client_paths:
    if any(part.startswith('.env') for part in relative.parts):
        raise SystemExit('Environment files must never be bundled')
    path = source / relative
    if path.is_symlink() or not path.is_file() or any(parent.is_symlink() for parent in path.parents):
        raise SystemExit('Client redirects/special files refused')
source_sha = subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip()
build_info = json.loads(subprocess.check_output([server, '--build-info'], text=True, timeout=10))
if build_info.get('commit') != source_sha:
    raise SystemExit('Server binary build commit must match packaging checkout; unknown/mismatched binary refused')
version = next(line.split('"')[1] for line in (source / 'server/Cargo.toml').read_text().splitlines() if line.startswith('version = '))
if build_info.get('version') != version:
    raise SystemExit('Server binary package version must match Cargo version')
toolchain_path = os.environ.get('FRAYD_NATIVE_TOOLCHAIN_FILE')
if toolchain_path:
    toolchain_file = Path(toolchain_path)
    if not toolchain_file.is_absolute() or toolchain_file.is_symlink() or not toolchain_file.is_file():
        raise SystemExit('Toolchain metadata must be an absolute regular file')
    toolchain = toolchain_file.read_text().strip()
else:
    toolchain = subprocess.check_output(['rustc', '+1.99.1', '--version', '--verbose'], text=True).strip()
if not toolchain.startswith('rustc ') or not any(line.startswith('host: ') for line in toolchain.splitlines()) or not any(line.startswith('release: ') for line in toolchain.splitlines()):
    raise SystemExit('Complete actual rustc --version --verbose metadata is required')
output.mkdir(mode=0o700)
bundle = output / f'frayd-{version}-{target}'
bundle.mkdir(mode=0o700)
shutil.copy2(server, bundle / 'frayd-server')
shutil.copy2(helper, bundle / 'frayd-native')
(bundle / 'build-info.json').write_text(json.dumps(build_info, indent=2) + '\n')
for relative in client_paths:
    destination = bundle / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source / relative, destination)
for name, relative in [('install.sh', 'scripts/native/install.sh'), ('start.sh', 'scripts/native/start.sh'), ('uninstall.sh', 'scripts/uninstall.sh')]:
    shutil.copy2(source / relative, bundle / name)
(bundle / 'README.md').write_text('''Frayd experimental native bundle

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
The manifest source SHA identifies the packaging checkout base commit. The
build-info.json is read from the actual server binary; dirty/unknown builds retain
that explicit state. Manifest hashes identify the packaged bytes. macOS archives are unsigned.
''')
for path in sorted(bundle.rglob('*')):
    path.chmod(0o755 if path.is_dir() or path.name in {'frayd-server', 'frayd-native', 'install.sh', 'start.sh', 'uninstall.sh'} and path.parent == bundle else 0o644)
entries = []
for path in sorted(bundle.rglob('*')):
    entries.append({'path': path.relative_to(bundle).as_posix(), 'kind': 'dir' if path.is_dir() else 'file', 'mode': stat.S_IMODE(path.stat().st_mode), 'size': path.stat().st_size if path.is_file() else 0, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else ''})
manifest = {'schema': 1, 'version': version, 'source': source_sha, 'target': target, 'toolchain': toolchain, 'entries': entries}
(bundle / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
archive = output / (bundle.name + '.tar.gz')
with tarfile.open(archive, 'w:gz') as tar:
    tar.add(bundle, arcname=bundle.name, recursive=True)
(archive.with_suffix(archive.suffix + '.sha256')).write_text(hashlib.sha256(archive.read_bytes()).hexdigest() + '  ' + archive.name + '\n')
print(archive)
PY
