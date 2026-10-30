"""Prove the Docker allowlist excludes synthetic state, secrets, editor and installer files."""
import io
import secrets
import tarfile
import tempfile
from pathlib import Path

from integration import cleanup_project, docker

if not __debug__:
    raise SystemExit('unsupported optimized Python execution: qualification requires assertions; remove -O and PYTHONOPTIMIZE')

ROOT = Path(__file__).resolve().parents[1]
SERVER_SOURCES = ('Cargo.toml', 'Cargo.lock', 'build.rs', 'schema.sql', 'src/main.rs', 'tests/archive_readers.rs')
STATE_SENTINELS = ('.env', '.env.local', '.env.production', '.env.production.local',
                   'frayd.db', 'frayd.db-wal', 'frayd.sqlite', 'frayd.sqlite-wal')


def write_fixture(path, text):
    """Create synthetic probe content exclusively; never replace an existing file."""
    with path.open('x', encoding='utf-8') as output:
        output.write(text)


def main():
    """Build and export a disposable scratch image containing only fake client and server files."""
    tag = 'frayd-context-qa:' + secrets.token_hex(6)
    container = None
    def cleanup(*_args):
        """Remove only this probe's image and exported container; always attempt both."""
        try:
            if container:
                docker('rm', '-f', container)
        finally:
            docker('image', 'rm', '-f', tag)

    with cleanup_project(cleanup):
        with tempfile.TemporaryDirectory(prefix='frayd-context-qa-') as directory:
            context = Path(directory)
            write_fixture(context / '.dockerignore', (ROOT / '.dockerignore').read_text(encoding='utf-8'))
            write_fixture(context / 'Dockerfile', 'FROM scratch\nCOPY . /\n')
            tests = context / 'tests'
            (tests / 'fixtures' / 'private').mkdir(parents=True)
            write_fixture(tests / 'fixtures/native-resume.mp4', 'normal synthetic generated video fixture')
            for name in ('test_other.py', 'private.txt', 'fixtures/other.mp4', 'fixtures/generate-native-resume.sh',
                         'fixtures/native-resume.mp4.orig', 'fixtures/.env', 'fixtures/frayd.sqlite',
                         'fixtures/private/secret.txt'):
                write_fixture(tests / name, 'NONSECRET_SYNTHETIC_SENTINEL_' + name)
            client = context / 'client'
            (client / 'install').mkdir(parents=True)
            (client / 'js').mkdir()
            write_fixture(client / 'js/app.js', 'normal synthetic client asset')
            for name in (*STATE_SENTINELS, 'notes.md', '.DS_Store', 'js/app.js.orig', 'js/app.js.swp'):
                write_fixture(client / name, 'NONSECRET_SYNTHETIC_SENTINEL_' + name)
            write_fixture(client / 'install/bootstrap.sh', 'NONSECRET_INSTALLER_SENTINEL')
            server = context / 'server'
            for name in SERVER_SOURCES:
                (server / name).parent.mkdir(parents=True, exist_ok=True)
                write_fixture(server / name, 'normal synthetic server source ' + name)
            for name in ('frayd.db', 'frayd.db-wal', 'notes.txt', 'thumbs/1.jpg', 'transcodes/a.mp4',
                         'target/x', 'src/.DS_Store', 'src/main.rs.orig'):
                (server / name).parent.mkdir(parents=True, exist_ok=True)
                write_fixture(server / name, 'NONSECRET_SYNTHETIC_SENTINEL_' + name)
            for allowed in (client / 'js', server / 'src'):
                for name in STATE_SENTINELS:
                    write_fixture(allowed / name, 'NONSECRET_SYNTHETIC_SENTINEL_' + name)
            docker('build', '-t', tag, directory)
        container = docker('create', tag, '/unused').stdout.decode().strip()
        exported = docker('export', container).stdout
        assert b'NONSECRET_SYNTHETIC_SENTINEL' not in exported, 'Synthetic state sentinel leaked into Docker context'
        assert b'NONSECRET_INSTALLER_SENTINEL' not in exported
        with tarfile.open(fileobj=io.BytesIO(exported)) as archive:
            fixture_files = sorted(m.name for m in archive.getmembers() if m.isfile() and m.name.startswith('tests/'))
            assert fixture_files == ['tests/fixtures/native-resume.mp4'], fixture_files
            assert archive.extractfile(fixture_files[0]).read() == b'normal synthetic generated video fixture'
            client_files = sorted(m.name for m in archive.getmembers() if m.isfile() and m.name.startswith('client/'))
            assert client_files == ['client/js/app.js'], client_files
            asset = archive.extractfile('client/js/app.js')
            assert asset.read() == b'normal synthetic client asset'
            server_files = sorted(m.name for m in archive.getmembers() if m.isfile() and m.name.startswith('server/'))
            assert server_files == sorted('server/' + name for name in SERVER_SOURCES), server_files
        print('PASS: Docker context keeps client/server sources and exactly one build-only fixture; excludes test neighbors, secrets, databases, other media, build, editor and installer files')


if __name__ == '__main__':
    main()
