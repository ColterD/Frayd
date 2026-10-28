"""Prove the Docker allowlist excludes synthetic environment and installer files."""
import io
import secrets
import shutil
import tarfile
import tempfile
from pathlib import Path

from integration import cleanup_project, docker

ROOT = Path(__file__).resolve().parents[1]


def main():
    """Build and export a disposable scratch image containing only fake client files."""
    tag = 'frayd-context-qa:' + secrets.token_hex(6)
    container = None
    def cleanup(*_args):
        """Remove only this probe's image and exported container."""
        if container:
            docker('rm', '-f', container)
        docker('image', 'rm', '-f', tag)

    with cleanup_project(cleanup):
        with tempfile.TemporaryDirectory(prefix='frayd-context-qa-') as directory:
            context = Path(directory)
            shutil.copyfile(ROOT / '.dockerignore', context / '.dockerignore')
            (context / 'Dockerfile').write_text('FROM scratch\nCOPY client/ /client/\n')
            client = context / 'client'
            (client / 'install').mkdir(parents=True)
            (client / 'js').mkdir()
            (client / 'js/app.js').write_text('normal synthetic client asset')
            for name in ('.env', '.env.local', '.env.production', '.env.production.local'):
                (client / name).write_text('NONSECRET_SYNTHETIC_SENTINEL_' + name)
            (client / 'install/bootstrap.sh').write_text('NONSECRET_INSTALLER_SENTINEL')
            docker('build', '-t', tag, directory)
        container = docker('create', tag, '/unused').stdout.decode().strip()
        exported = docker('export', container).stdout
        with tarfile.open(fileobj=io.BytesIO(exported)) as archive:
            names = archive.getnames()
            assert not any('/.env' in name or name.startswith('client/install') for name in names), names
            asset = archive.extractfile('client/js/app.js')
            assert asset.read() == b'normal synthetic client asset'
        assert b'NONSECRET_SYNTHETIC_SENTINEL' not in exported
        assert b'NONSECRET_INSTALLER_SENTINEL' not in exported
        print('PASS: real Docker context excludes .env family and installers; normal client asset survives')


if __name__ == '__main__':
    main()
