# Frayd

Frayd is a self-hosted media library with a Rust server (axum and SQLite) and a plain JavaScript browser client. It scans local videos, music, audiobooks, ebooks, comics and manga. Features include saved progress and watch parties.

This is an early-stage project. Start on your own computer and use trusted media. The Docker setup below publishes only to loopback; it does not configure public hosting or household LAN access.

## Run with Docker

Requirements: Git, Docker Engine and the Docker Compose plugin. Run these commands from a terminal:

```sh
git clone https://github.com/kattiest/Frayd.git
cd Frayd
docker compose config --quiet
docker compose up -d --build --wait
```

Open **http://127.0.0.1:3100**. The first account created becomes the owner/admin. You can also continue as a guest. Logging in with an unused email creates another account; this is not an invite-only account system. Guest access and library/media routes are not a private authentication boundary. Keep this deployment on a trusted machine and do not expose it directly to the internet.

If port 3100 is occupied, prefix Compose commands with `FRAYD_PORT=3105`, then open http://127.0.0.1:3105. This variable changes the published proxy port; the app still listens on port 8484 inside Docker.

The first build downloads images and Rust dependencies, then runs locked Rust tests, Clippy and the release build. It can take several minutes. Runtime services run as UID/GID 10001 with read-only root filesystems, dropped capabilities, resource limits and temporary `/tmp` storage. Only the proxy publishes a port. The app is on an internal Docker network without external routing.

### Add your media

By default, the named `data` volume holds the database, thumbnails and `/data/library`. To use an existing host media folder without copying it, create `compose.override.yaml` beside `compose.yaml`:

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

The app sees your files at `/data/library`, not their host path. The image sets `FRAYD_LIBRARY=/data/library`; this takes precedence over the library path saved in Settings. Change the mount source to select a different host folder. Choose **Scan library** after adding or changing media. The read-only media mount prevents the app from changing those source files. The database and thumbnails remain in the `data` volume.

Alternatively, copy files into the default library volume (this uses additional disk space):

```sh
docker compose cp /absolute/path/to/your/media/. app:/data/library/
```

### Persistence, stopping and removal

`/data/frayd.sqlite` stores accounts, sessions, library metadata and reading/playback progress. `/data/thumbs` holds generated thumbnails. These survive container recreation in the Compose `data` volume. Back up that volume with the app stopped for a consistent database copy; media bind mounts need their own backup.

```sh
docker compose ps
docker compose stop            # stop services, keep containers and data
docker compose start           # resume services
docker compose down            # remove containers and networks, keep data
```

To remove the deployment **and permanently delete its named volume**, including accounts, progress, thumbnails and any media copied into it:

```sh
docker compose down --volumes
```

A separately bind-mounted host media directory is not removed by that command. Keep the same Compose project name/directory when restarting if you want to reuse the same volume.

## Supported tools and current limits

- The Docker image includes `ffmpeg` and `ffprobe` for metadata, thumbnails and transcoding. Actual playback still depends on the media format and browser codec support.
- ZIP-based CBZ/EPUB handling is built into the server. RAR-based CBR needs the optional `unrar` executable, which this image does not include.
- Automatic router port mapping needs optional `upnpc`. It is not included, and the image defaults `FRAYD_UPNP=0`. The Compose example supplies neither TLS nor automatic network exposure.
- Outside the isolated Docker network, audio thumbnail fallback can send filename-derived search terms to Apple's iTunes Search API and download artwork. Local metadata/embedded artwork extraction runs first. The supplied Compose app has no external routing, so online artwork lookup is unavailable and a missing thumbnail is expected.
- Videos are currently classified as movies; season/episode organization is not implemented.
- The browser installer pages and bootstrap scripts are experimental; the Docker commands above are the reproducible startup route documented here.
- Dependency audit is currently red for known advisories affecting locked `block-buffer` and `zip` versions. Run the audit below for current advisory IDs. Container isolation does not establish that arbitrary media is safe.
- No license file is currently present in the repository. Do not assume redistribution terms from the public repository alone.

## Development and verification

For a native server, install a current Rust toolchain, Clippy and a C compiler. Keep `client/` beside `server/`: the server embeds that source path at build time. Run from `server/`:

```sh
cargo test --locked
cargo clippy --locked --all-targets
cargo build --locked --release
FRAYD_LIBRARY=/absolute/path/to/your/media cargo run --locked
```

The native default port is 8484. Environment overrides include `FRAYD_DB`, `FRAYD_LIBRARY`, `FRAYD_THUMBS`, `FRAYD_PORT`, `FRAYD_UPNP`, and the optional PEM paths `FRAYD_CERT`/`FRAYD_KEY` for TLS. Native runs do not have the Compose network isolation.

From the repository root, with Node.js and Python 3.11+:

```sh
node --test tests/client.test.cjs
docker compose config --quiet
docker compose build
python3 scripts/integration.py
python3 scripts/audit-dependencies.py
```

The integration script uses already-built `frayd:local` and `frayd-proxy:local` images, a unique disposable project, a randomly assigned loopback port and a synthetic comic. It checks the page, first-owner setup, scan, comic and range bytes, saved progress, container recreation and logout. It removes its own project and volume in a `finally` block. It never targets the normal `frayd` project. It does not prove browser playback, every media format, watch-party behavior or public-hosting security.

The dependency audit queries OSV for exact versions in `server/Cargo.lock`, prints its findings and exits nonzero on advisories. Network/API failures also fail the command. CI runs this as a separate check, without suppressing known findings. No application dependencies are added by these verification scripts.
