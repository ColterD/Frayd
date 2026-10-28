# Frayd

Frayd is a self-hosted media library with a Rust server (axum and SQLite) and a plain JavaScript browser client. It scans local videos, music, audiobooks, ebooks, comics and manga. Features include saved progress and watch parties.

This is an early-stage project. Start on your own computer and use trusted media. The Docker setup below publishes only to loopback; it does not configure public hosting or household LAN access.

## Run with Docker

Requirements: Git, Docker Engine and the Docker Compose plugin. These commands target the fork draft under verification; the [upstream repository](https://github.com/kattiest/Frayd) does not yet contain all of this branch's fixes. Run these commands from a terminal:

```sh
git clone --branch fix/coderabbit-all-findings --single-branch https://github.com/ColterD/Frayd.git
cd Frayd
docker compose config --quiet
docker compose up -d --build --wait
```

Open **http://127.0.0.1:3100**. The first account created becomes the owner/admin. You can also continue as a guest. Logging in with an unused email creates another account; this is not an invite-only account system. Guest access and library/media routes are not a private authentication boundary. Keep this deployment on a trusted machine and do not expose it directly to the internet.

If port 3100 is occupied, prefix Compose commands with `FRAYD_PORT=3105`, then open http://127.0.0.1:3105. This variable changes the published proxy port; the app still listens on port 8484 inside Docker.

The first build downloads images and Rust dependencies, then runs locked Rust tests, Clippy and the release build. It can take several minutes. Runtime services run as UID/GID 10001 with read-only root filesystems, dropped capabilities, resource limits and temporary `/tmp` storage. Only the proxy publishes a port. The app is on an internal Docker network without external routing.

### Add your media

By default, the named `data` volume holds the database, thumbnails, transcode cache and `/data/library`. To use an existing host media folder without copying it, create `compose.override.yaml` beside `compose.yaml`:

```yaml
services:
  app:
    volumes:
      - type: bind
        source: /absolute/path/to/your/media
        target: /data/library
        read_only: true
        bind:
          create_host_path: false
```

Replace the source with a real directory, ensure UID 10001 can read its files and traverse its directories, then run:

```sh
docker compose config --quiet
docker compose up -d --wait
```

The app sees your files at `/data/library`, not their host path. The image sets `FRAYD_LIBRARY=/data/library`; this takes precedence over the library path saved in Settings. Change the mount source to select a different host folder. Choose **Scan library** after adding or changing media. The read-only media mount prevents the app from changing those source files. The database, thumbnails and converted playback files remain in the writable `data` volume, outside the source library.

Alternatively, copy files into the default library volume (this uses additional disk space):

```sh
docker compose cp /absolute/path/to/your/media/. app:/data/library/
```

### Persistence, stopping and removal

`/data/frayd.sqlite` stores accounts, sessions, library metadata and reading/playback progress. `/data/thumbs` holds generated thumbnails. `/data/transcodes/<media-id>/<job-UUID>/` holds converted playback files. These survive container recreation in the Compose `data` volume. Back up that volume with the app stopped for a consistent database copy; media bind mounts need their own backup.

```sh
docker compose ps
docker compose stop            # stop services, keep containers and data
docker compose start           # resume services
docker compose down            # remove containers and networks, keep data
```

To remove the deployment **and permanently delete its named volume**, including accounts, progress, thumbnails, converted playback files and any media copied into it:

```sh
docker compose down --volumes
```

A separately bind-mounted host media directory is not removed by that command. Keep the same Compose project name/directory when restarting if you want to reuse the same volume.

## Supported tools and current limits

- The Docker image includes `ffmpeg` and `ffprobe` for metadata, thumbnails and transcoding. Actual playback still depends on the media format and browser codec support.
- ZIP-based CBZ/EPUB handling is built into the server. RAR-based CBR needs the optional `unrar` executable, which this image does not include.
- Automatic router port mapping needs optional `upnpc`. It is not included, and the image defaults `FRAYD_UPNP=0`. The Compose example supplies neither TLS nor automatic network exposure.
- Outside the isolated Docker network, audio thumbnail fallback can send filename-derived search terms to Apple's iTunes Search API and download artwork. Local metadata/embedded artwork extraction runs first. The supplied Compose app has no external routing, so online artwork lookup is unavailable and a missing thumbnail is expected.
- YouTube watch parties load the YouTube iframe API in each participant's browser. Each browser needs access to YouTube; the app's isolated Docker network does not block browser-side requests. This feature has not been verified by the disposable integration checks below.
- **Add from URL** uses an optional server-side `yt-dlp` executable to download into the library. The supplied image does not include it, and the Compose app has no external routing, so URL ingestion is unavailable in this setup. Native use additionally requires network access and a writable library; the read-only media mount above cannot receive downloads.
- Videos are currently classified as movies; season/episode organization is not implemented.
- The browser installer pages and bootstrap scripts are experimental; the Docker commands above are the reproducible startup route documented here.
- ZIP readers support stored and deflate archives. Container isolation does not establish that arbitrary media is safe.
- No license file is currently present in the repository. Do not assume redistribution terms from the public repository alone.

## Development and verification

For a native server, install a current Rust toolchain, Clippy and a C compiler. Keep `client/` beside `server/`: the server embeds that source path at build time. Run from `server/`:

```sh
cargo test --locked
cargo clippy --locked --all-targets
cargo build --locked --release
FRAYD_LIBRARY=/absolute/path/to/your/media cargo run --locked
```

The native default port is 8484. Environment overrides include `FRAYD_DB`, `FRAYD_LIBRARY`, `FRAYD_THUMBS`, `FRAYD_TRANSCODES`, `FRAYD_PORT`, `FRAYD_UPNP`, and the optional PEM paths `FRAYD_CERT`/`FRAYD_KEY` for TLS. When `FRAYD_TRANSCODES` is unset, converted files go in `transcodes/` beside the database, under a media ID directory with a unique UUID directory for each conversion job; keep that cache writable and outside any read-only source library. Docker explicitly uses `/data/transcodes`. Existing converted media references are preserved for unchanged sources. Older `.frayd-tmp` files in a source library are not automatically migrated or removed; inspect and back up legacy files before deciding what to remove. Native runs do not have the Compose network isolation.

On rescan, a detectable change in a known source timestamp or byte length invalidates its current conversion reference. Conversion jobs also check the actual file version and database snapshot before publishing. Legacy references with initially unknown fingerprints are kept for compatibility; their original source version remains unverified. Rescan fingerprints use seconds plus byte length, so replacements preserving both values are undetected. Older completed cache files and ownership records remain until manual cleanup, so cache usage can grow across replacements. Back up data and inspect current playback references before removing historical outputs.

From the repository root, with Node.js and Python 3.11+:

```sh
node --test tests/*.test.cjs
docker compose config --quiet
docker build -t frayd-pr2-qa:local .
docker build -f Dockerfile.proxy -t frayd-pr2-proxy-qa:local .
python3 scripts/check-build-context.py
python3 scripts/integration.py
python3 -m unittest discover -s tests -p 'test_*.py'
python3 scripts/audit-dependencies.py
```

The integration script defaults to the already-built `frayd-pr2-qa:local` and `frayd-pr2-proxy-qa:local` images (`--app-image` / `--proxy-image` can override them). It uses a unique disposable project, a randomly assigned loopback port and synthetic media. It checks first-owner setup, scan, comic and Range bytes, saved progress, recreation and logout. It also reserves the app's old IP on its own network, recreates only the app and requires HTTP recovery with the proxy container and start time unchanged. A blocked `ffprobe` test requires health, static root, library and progress requests to remain responsive while scanning a read-only library, then releases the probe and requires scan completion. It converts an actual H.264/AC3 fixture to H.264/AAC in the writable cache, checks unchanged source bytes and directory contents, and verifies exact converted HTTP bytes survive an app restart. It then deliberately replaces only that disposable source clip with distinct bytes, size and mtime, requires a new conversion UUID and stream instead of stale cached bytes, and preserves the media ID and saved progress across rescan and restart. Existing completed cache files may remain. Docker health checks use the database-independent `/api/health`; the app also checks its static root. The context check builds only synthetic client files and proves environment files and installer assets are excluded.

These checks remove their own containers, networks and volume, preserve the original command failure if cleanup also fails, and never target the normal `frayd` project. They do not prove browser playback, every media format, watch-party behavior or public-hosting security. The app runs as UID/GID 10001; existing host data and custom cache paths must grant that user the required access. Read-only source mounts protect source bytes, while generated data requires writable space.

The dependency audit queries OSV for exact versions in `server/Cargo.lock`, prints its findings and exits nonzero on applicable advisories. It records one exact database disagreement: OSV flags `block-buffer 0.10.4` for `GHSA-qwgh-2vcv-g2f7`, while the publisher lists only `>=0.11.0,<0.12.1`. Each audit fetches the publisher advisory from the RustCrypto GitHub API and requires that exact identity, package and range before reporting this finding as `not_affected`. Changed, malformed or unavailable publisher evidence keeps the finding fatal. All other findings remain fatal. OSV network/API failures also fail the command. CI tests this narrow policy and runs the live query as a separate check. No application dependencies are added by these verification scripts.
