# Frayd

![CodeRabbit Pull Request Reviews](https://img.shields.io/coderabbit/prs/github/ColterD/Frayd?utm_source=oss&utm_medium=github&utm_campaign=ColterD%2FFrayd&labelColor=171717&color=FF570A&link=https%3A%2F%2Fcoderabbit.ai&label=CodeRabbit+Reviews)

Frayd is a self-hosted media library with a Rust server (axum and SQLite) and a plain JavaScript browser client. It scans local movies, TV episodes, music, audiobooks, ebooks, comics and manga, with series/author navigation, saved progress and watch parties.

Implementation snapshot: [`6a4d202`](https://github.com/ColterD/Frayd/tree/6a4d2023d52d57fe3957c6da7827ea78813a0737). See [PR #2](https://github.com/ColterD/Frayd/pull/2) for current deployment and acceptance status. This is an early-stage project. Start on your own computer with trusted media. The Docker setup publishes only to loopback; it does not configure household LAN access or public hosting.

## Run with Docker

Requirements: Git, Python 3, Docker Engine and the Docker Compose plugin. These commands target the fork draft under verification; the [upstream repository](https://github.com/kattiest/Frayd) does not yet contain all of this branch's fixes.

```sh
git clone --branch fix/coderabbit-all-findings --single-branch https://github.com/ColterD/Frayd.git
cd Frayd
docker compose config --quiet
python3 scripts/build-metadata.py -- docker compose up -d --build --wait
```

Open **http://127.0.0.1:3100**. If port 3100 is occupied, prefix Compose commands with `FRAYD_PORT=3105` and open http://127.0.0.1:3105. The first build downloads images and Rust dependencies and runs tests, Clippy and a release build; it can take several minutes.

The sign-in screen and sidebar show the running server's version and commit. See [checking a build against upstream](docs/operations.md#check-a-build-against-upstream) to inspect a custom build or check for upstream changes.

The dashboard labels all seven media categories and starts with newest items first; each category can switch to oldest first. Artwork comes from local media when available, with a symbol fallback when no usable cover exists.

The first account becomes the owner/admin. You can continue as a guest, and logging in with an unused email creates another account. This is not invite-only: **guest access and library/media routes are not a private authentication boundary**. Keep the deployment on a trusted machine and do not expose it directly to the internet. Saving ebook highlights requires a signed-in account. Sign-in uses local email/password; OAuth/SSO and notifications are not implemented.

## Add your first library

Follow [the read-only host-library mount](docs/operations.md#add-a-host-library), then choose **Scan library**. The app sees `/data/library`; its environment setting overrides the path in Settings. The app runs as UID/GID 10001 and needs permission to read and traverse the host media.

The default named volume stores accounts, progress, thumbnails, conversions and any media copied into it. These survive container recreation. **`docker compose down --volumes` permanently deletes that data**, including copied media. See [storage and removal](docs/operations.md#stop-resume-and-remove) before changing or removing the deployment. See the [bounded same-image recovery qualification and unqualified upgrade status](docs/operations.md#backup-restore-and-upgrade-status).

## Documentation

- [Documentation index](docs/README.md): guides by task.
- [Operations](docs/operations.md): media mounts, permissions, storage, cache limits, stopping/removal and troubleshooting.
- [Development](docs/development.md): native setup, canonical checks, disposable QA and dependency-audit policy.
- [Verification ledger](docs/verification/coderabbit-follow-up.md): historical PR findings and receipts, with source/image boundaries and remaining acceptance gates.

Playback depends on format and browser support; optional tools and network-dependent features have [current limits](docs/operations.md#supported-tools-and-current-limits). Browser installers and bootstrap scripts are experimental. No license file is currently present; do not assume redistribution terms from the public repository alone.
