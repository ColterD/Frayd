"""Prove the Docker allowlist excludes synthetic state, secrets, editor and installer files."""
import io
import secrets
import shutil
import tarfile
import tempfile
from pathlib import Path

from integration import cleanup_project, docker

ROOT = Path(__file__).resolve().parents[1]
SERVER_SOURCES = ('Cargo.toml', 'Cargo.lock', 'build.rs', 'schema.sql', 'src/main.rs', 'tests/archive_readers.rs')


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
            shutil.copyfile(ROOT / '.dockerignore', context / '.dockerignore')
            (context / 'Dockerfile').write_text('FROM scratch\nCOPY client/ /client/\nCOPY server/ /server/\n')
            client = context / 'client'
            (client / 'install').mkdir(parents=True)
            (client / 'js').mkdir()
            (client / 'js/app.js').write_text('normal synthetic client asset')
            for name in ('.env', '.env.local', '.env.production', '.env.production.local',
                         'frayd.db', 'frayd.db-wal', 'notes.md', '.DS_Store', 'js/app.js.orig', 'js/app.js.swp'):
                (client / name).write_text('NONSECRET_SYNTHETIC_SENTINEL_' + name)
            (client / 'install/bootstrap.sh').write_text('NONSECRET_INSTALLER_SENTINEL')
            server = context / 'server'
            for name in SERVER_SOURCES:
                (server / name).parent.mkdir(parents=True, exist_ok=True)
                (server / name).write_text('normal synthetic server source ' + name)
            for name in ('frayd.db', 'frayd.db-wal', 'notes.txt', 'thumbs/1.jpg', 'transcodes/a.mp4',
                         'target/x', 'src/.DS_Store', 'src/main.rs.orig'):
                (server / name).parent.mkdir(parents=True, exist_ok=True)
                (server / name).write_text('NONSECRET_SYNTHETIC_SENTINEL_' + name)
            docker('build', '-t', tag, directory)
        container = docker('create', tag, '/unused').stdout.decode().strip()
        exported = docker('export', container).stdout
        with tarfile.open(fileobj=io.BytesIO(exported)) as archive:
            client_files = sorted(m.name for m in archive.getmembers() if m.isfile() and m.name.startswith('client/'))
            assert client_files == ['client/js/app.js'], client_files
            asset = archive.extractfile('client/js/app.js')
            assert asset.read() == b'normal synthetic client asset'
            server_files = sorted(m.name for m in archive.getmembers() if m.isfile() and m.name.startswith('server/'))
            assert server_files == sorted('server/' + name for name in SERVER_SOURCES), server_files
        assert b'NONSECRET_SYNTHETIC_SENTINEL' not in exported
        assert b'NONSECRET_INSTALLER_SENTINEL' not in exported
        print('PASS: Docker context keeps client and server sources and excludes env, database, media, build, editor and installer files')


if __name__ == '__main__':
    main()
