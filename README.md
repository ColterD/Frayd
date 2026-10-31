# Frayd

Frayd is a self-hosted media library with a Rust server (axum and SQLite) and a plain JavaScript browser client. It scans local movies, TV episodes, music, audiobooks, ebooks, comics and manga, with series and author navigation, saved progress and watch parties.

This is an early-stage project. Start on your own computer with trusted media. The Docker setup publishes only to loopback. It does not configure household LAN access or public hosting.


## Run with Docker

Requirements: Git, Python 3, Docker Engine and the Docker Compose plugin.

```sh
git clone https://github.com/kattiest/Frayd.git
cd Frayd
docker compose config --quiet
python3 scripts/build-metadata.py -- docker compose up -d --build --wait
```

Open **http://127.0.0.1:3100**. If port 3100 is occupied, prefix Compose commands with `FRAYD_PORT=3105` and open http://127.0.0.1:3105. The first build downloads images and Rust dependencies and runs tests, Clippy and a release build, so it can take several minutes.

URL downloads are disabled in the isolated default image. To enable direct HTTP(S) MP4 video downloads and outbound routing on the same image, follow [opt-in Docker downloads](docs/operations.md#opt-in-docker-downloads).

The sign-in screen and sidebar show the running server's version and commit. To inspect a custom build or check for upstream changes, see [check a build against upstream](docs/operations.md#check-a-build-against-upstream).

The dashboard labels all seven media categories and starts with the newest items first. Each category can switch to oldest first. Artwork comes from local media when available, with a symbol fallback when no usable cover exists.

The first account becomes the owner and administrator. You can continue as a guest, and logging in with an unused email creates another account. Sign-up is open, and **guest access, library routes and media routes are not a private authentication boundary**. Keep the deployment on a trusted machine and do not expose it directly to the internet. Saving ebook highlights requires a signed-in account. Sign-in uses a local email and password. OAuth, SSO and notifications are not implemented.

## Add your first library

Follow [the read-only host-library mount](docs/operations.md#add-a-host-library), then choose **Scan library**. The app sees `/data/library`, and its environment setting overrides the path in Settings. The app runs as UID and GID 10001 and needs permission to read and traverse the host media.

The default named volume stores accounts, progress, thumbnails, conversions and any media copied into it. These survive container recreation. **`docker compose down --volumes` permanently deletes that data**, including copied media. Read [stop, resume and remove](docs/operations.md#stop-resume-and-remove) before changing or removing the deployment, and [backup and restore](docs/operations.md#backup-and-restore) before you rely on one.

## Documentation

- [Documentation index](docs/README.md): guides by task.
- [Changelog](CHANGELOG.md): unreleased product changes.
- [Operations](docs/operations.md): media mounts, permissions, storage, cache limits, stopping and removal, backup and troubleshooting.
- [Development](docs/development.md): native setup, the checks CI runs, disposable QA and dependency-audit policy.
- [Native bundle](docs/native-bundle.md): the experimental private native install.

Playback depends on format and browser support. Optional tools and network-dependent features have [current limits](docs/operations.md#supported-tools-and-current-limits). Browser installers and bootstrap scripts are experimental. No license file is present, so do not assume redistribution terms from the public repository alone.
