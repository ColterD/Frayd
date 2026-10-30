# Operations

[Documentation index](README.md) · [Quick start](../README.md) · [Development and checks](development.md)

These instructions cover the Docker setup. Use trusted media on a trusted machine. Guest access, library routes and media routes are not a private authentication boundary, and Compose binds only to loopback with no TLS or public hosting. Guests keep playback and reading progress per device. Saving ebook highlights requires a signed-in account, and a guest device identifier is not an account credential.

## Add a host library

Create `compose.override.yaml` beside `compose.yaml` to use existing media without copying it:

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

Replace the source with a real directory. The app runs as UID and GID 10001, so it needs read access to files and traversal access to directories. Keep the source mount read-only: the database, thumbnails and conversions need separate writable storage. Do not solve a permission failure by making the source library writable.

```sh
docker compose config --quiet
docker compose up -d --wait
```

The app sees `/data/library`, not the host path. The image's nonempty `FRAYD_LIBRARY=/data/library` selects that single exclusive root and locks library changes in Settings. Change the mount source to select a different host folder, then choose **Scan library** after adding or changing media.

### Configure multiple roots

The scanner, ingestion and cache checks share one ordered list of library roots. `FRAYD_LIBRARY` decides where that list comes from:

- With `FRAYD_LIBRARY` absent or empty, administrator Settings reads the database `library_paths`. A legacy `library_path` is used only when `library_paths` is absent, and a successful root update migrates it. An explicit `[]` disables the library, with no hidden fallback. Existing directories are canonicalized and deduplicated, keeping first-occurrence order.
- A nonempty `FRAYD_LIBRARY` selects exactly one root. It does not add to the database roots, and root edits return 409 while it is set.

Docker's image default selects `/data/library`. To use database-managed roots, explicitly clear it in the destination configuration. For example, add these read-only mounts in `compose.override.yaml`, then select `/media/books` and `/media/music` in administrator Settings:

```yaml
services:
  app:
    environment:
      FRAYD_LIBRARY: ''
    volumes:
      - type: bind
        source: /absolute/path/to/books
        target: /media/books
        read_only: true
        bind:
          create_host_path: false
      - type: bind
        source: /absolute/path/to/music
        target: /media/music
        read_only: true
        bind:
          create_host_path: false
```

Paths entered in Settings are container paths. The sources must already exist and be readable and traversable by UID and GID 10001. If you enable URL ingestion (see the limits below), its landing directory is `Internet/` under the first root, and that root must be writable. Choose a separate writable root first and keep original read-only media mounts read-only.

Root changes follow these rules:

- Removing a configured root keeps its indexed IDs, progress, annotations, thumbnails and source bytes.
- A root that was valid and is unavailable at startup stays configured. Health, administrator settings and status keep working, and the configuration count and metadata stay intact. Scan and cache work is held, and ingestion returns 503 when its media tools are available. The server does not recreate missing media roots.
- A newly submitted root that does not exist returns 400.
- A settings change is validated in full before one atomic database transaction. Malformed settings return 400 before any busy refusal. A valid root edit returns 409 while a root lock is set or a scan, conversion or ingestion job is active. The conversion worker counts as active while it only checks whether a file needs converting, so retry the edit later. A database failure returns 500 and publishes no partial settings.

Scan admission uses one compare-and-set worker owner with scope-bound cleanup. Successful downloads queue indexing through that worker and become `done` only after indexing completes; an abandoned download becomes `failed`. Status exposes `scanning`, `library_configured`, `library_count` and `ingesting` (`null`, `downloading`, `indexing`, `done` or `failed`). Public `/api/health` returns exactly `{"status":"ok"}` without root paths. Server version and effective paths belong to administrator settings metadata.

Alternatively, copy regular files and directories into a new subdirectory of the default library volume, using additional disk space. Symlinks, including any source path component, and special files are rejected without following them. Keep the source unchanged throughout the preflight and copy: these steps are not atomic. Run this block in Bash with GNU `find` and GNU `tar`, set the absolute source directory, and choose a new import name using only letters, digits, underscores or hyphens:

```sh
(
  set -eu
  set -o pipefail
  SOURCE_DIR=/absolute/path/to/your/media
  IMPORT_NAME=import-001
  case "$SOURCE_DIR" in /*) ;; *) echo 'Source must be absolute' >&2; exit 1 ;; esac
  while [[ "$SOURCE_DIR" != / && "$SOURCE_DIR" == */ ]]; do
    SOURCE_DIR=${SOURCE_DIR%/}
  done
  SOURCE_COMPONENT_PATH=/
  SOURCE_REMAINDER=${SOURCE_DIR#/}
  while [[ -n "$SOURCE_REMAINDER" ]]; do
    SOURCE_COMPONENT=${SOURCE_REMAINDER%%/*}
    case "$SOURCE_REMAINDER" in
      */*) SOURCE_REMAINDER=${SOURCE_REMAINDER#*/} ;;
      *) SOURCE_REMAINDER= ;;
    esac
    SOURCE_COMPONENT_PATH="${SOURCE_COMPONENT_PATH%/}/$SOURCE_COMPONENT"
    test ! -L "$SOURCE_COMPONENT_PATH" || { echo 'Source path must not contain symlinks' >&2; exit 1; }
  done
  test ! -L "$SOURCE_DIR"
  test -d "$SOURCE_DIR"
  UNSUPPORTED_ENTRY=$(find -P "$SOURCE_DIR" ! -type f ! -type d -printf x -quit)
  test -z "$UNSUPPORTED_ENTRY" || { echo 'Source must contain only regular files and directories' >&2; exit 1; }
  case "$IMPORT_NAME" in ''|*[!A-Za-z0-9_-]*) echo 'Invalid import name' >&2; exit 1 ;; esac
  docker compose exec -T app sh -eu -c '
    umask 077
    mkdir -- "/data/library/$1"
  ' sh "$IMPORT_NAME"
  tar --mode='u=rwX,go=' -C "$SOURCE_DIR" -cf - . |
    docker compose exec -T app sh -eu -c '
      umask 077
      exec tar --no-same-owner -xf - -C "/data/library/$1"
    ' sh "$IMPORT_NAME"
)
```

Use this alternative with the app running and without the read-only library override. GNU tar sets private archive-header modes with owner read and write access and directory traversal; it does not change the source permissions. This also handles sources readable only through group access. Extraction runs as the app's UID and GID 10001 with a private umask, so copied files remain readable and directories traversable by the app without granting group or other access. The new destination must not already exist. A failure can leave a partial import: inspect that directory before scanning or retrying. After a successful copy, choose **Scan library**. Media copied into the named volume shares that volume's deletion and backup lifecycle.

## Configuration and storage

Audio classification uses the nearest recognized music or audiobook folder name,
looking at most six ancestors and stopping at the selected library root. Whole
words and plurals distinguish names such as `Books` from `Booker T`. Without a
folder hint, `.m4b` and chapter-style and part-style filenames select audiobook; other
supported audio formats default to music. This is a naming heuristic, not content
recognition. Rescanning can correct an unchanged file's kind without resetting its
ID, progress, annotations, metadata or conversion references.

Compose publishes `127.0.0.1:3100`. If that port is occupied, prefix Compose commands with `FRAYD_PORT=3105` and open http://127.0.0.1:3105. This changes the proxy's published port, and the app still listens on 8484 inside Docker. Keep the same Compose project name and directory to reuse the same named volume.

| Docker path | Contents |
|---|---|
| `/data/frayd.sqlite` | Accounts, sessions, library metadata and reading and playback progress |
| `/data/library` | Default library, or the read-only host mount above |
| `/data/thumbs` | Generated thumbnails |
| `/data/transcodes/<media-id>/<job-UUID>/` | Converted playback files |

The Compose `data` volume persists database, thumbnails, conversions and any copied media across container recreation. A host library mount has its own storage lifecycle. Runtime services use read-only root filesystems, dropped capabilities, resource limits and temporary `/tmp` storage. Only the proxy publishes a port; the app's internal network has no external routing. See [compose.yaml](../compose.yaml) for exact limits.

Login and first-account setup share a budget of 10 attempts per client in five minutes; password changes have a separate budget of the same size. A client is its IPv4 address or its IPv6 /64, so devices sharing one IPv6 /64 share a budget. By default the client is the connection's peer address, and any `X-Forwarded-For` header is ignored. With `FRAYD_TRUSTED_PROXY=1` the server uses the last `X-Forwarded-For` entry, which the proxy wrote. Compose sets it on the app because the bundled proxy replaces that header with the real client address. Set it only when every request reaches the server through such a proxy; otherwise a client can choose its own rate-limit key. A Compose override that publishes the app port directly must also unset it.

Behind a tunnel or proxy that connects to a loopback server, the peer address is the tunnel's local address. Without `FRAYD_TRUSTED_PROXY`, every client then shares one rate-limit budget. If the tunnel or proxy overwrites or appends `X-Forwarded-For`, set `FRAYD_TRUSTED_PROXY=1`. If it passes a client-supplied header through untouched, leave the variable unset.

Environment overrides are `FRAYD_DB`, `FRAYD_LIBRARY`, `FRAYD_THUMBS`, `FRAYD_TRANSCODES`, `FRAYD_PORT`, `FRAYD_BIND`, `FRAYD_CLIENT_DIR`, `FRAYD_UPNP`, `FRAYD_TRUSTED_PROXY`, and optional PEM paths `FRAYD_CERT` and `FRAYD_KEY` for native TLS. Container paths must exist inside the container and have the required access. Compose reads the host `FRAYD_PORT` for publishing; other host variables are not automatically passed to the app. The [Dockerfile](../Dockerfile) sets its defaults. Native defaults and checks are in [Development](development.md#native-server).

### Administrator deletion

Administrator media deletion returns 204 only for an unreferenced database row with no retained history, conversion reference, provenance or active conversion job. Conflicting references or history return 409. Deletion also returns 409 while the conversion worker holds its slot, including while it only checks whether a file needs converting, which a scan can start for any unconverted video. That 409 carries `Retry-After: 1`, changes nothing and is safe to retry later; a referenced item returns 409 without `Retry-After`, even during that window. Deletion leaves original source bytes intact; a later scan can index that source again. Optional owned-thumbnail cleanup has its own conservative boundary and does not authorize source removal.

Scans generate optional JPEG covers from local video frames, attached audio art, declared EPUB covers and the first readable comic or manga image. Media without usable embedded art, including the public-domain audio samples, gets a symbol fallback.

- New covers use per-media UUID names. A source replacement invalidates the old pointer. After the database commit, cleanup removes a generated file only when it is proven and unreferenced.
- Media deletion follows the same rule. It keeps the original source, the selected legacy files and any file it cannot prove it owns.
- A crash between publishing or invalidating a cover and cleaning it up can leave an orphan. There is no general thumbnail reaper.
- Thumbnail responses identify legacy PNG, GIF, WebP and JPEG bytes by content and require revalidation. Legacy cached art is sniffed from 12 bytes and streamed in 64 KiB chunks, with full Content-Length and HEAD behavior.
- The library and search API returns only a nullable opaque `artwork_version` basename for a matching media ID, never the filesystem path. The client encodes it in the thumbnail URL so a new cover does not reuse an old immutable response.

Administrator account deletion removes the target account and its dependent records in one transaction, preserving other users' data. Self-deletion and deletion of the last administrator return 409. Database failures return 500; a failed transaction leaves the account and children intact.

### Conversion cache limits

Keep generated data writable and outside read-only source media. When `FRAYD_TRANSCODES` is unset, conversions go in `transcodes/` beside the database; Docker explicitly uses `/data/transcodes`.

Unchanged sources keep their conversion references. A rescan invalidates the current reference when it detects a changed source timestamp or byte length, and a conversion job checks the file and database versions before it publishes.

- Legacy references with unknown fingerprints keep working, but their original source version is unverified.
- Rescan fingerprints use whole seconds plus byte length, so a replacement that keeps both values goes undetected.
- When a directory walk or manga probe fails, the scan logs a warning with the operation and the path, if known, and continues through the entries it can read. Check the container logs for skipped paths, because a completed scan does not mean every subtree was readable.

A known source replacement can retire its exact inactive, unreferenced prior generated pair in the same transaction that clears playback, and a later recovery pass on Linux can delete those files and their ownership records. Other historical completed outputs and unknown records stay under operator control, so cache usage can keep growing. Inspect current references before you remove historical outputs. Older `.frayd-tmp` files in source libraries are neither migrated nor removed, so inspect and back them up before you decide what to delete. There is no general cache-pruning procedure. Do not rely on the generated cache for recovery: back up the original media and the persistent application data.

### Interrupted conversions

New conversion reservations record a pending, completed or retiring state. Where the platform supports file locking (Linux), a worker and its ffmpeg child hold the job lease. Startup, explicit rescans and the next conversion attempt can reclaim a pending or retiring job only when it is proven inactive and eligible. If the database blocks a cleanup, the bytes and provenance stay and a later entry point retries it.

Publication marks completion atomically with the playback pointer. A known source replacement can move the eligible prior completed pair to retiring for later recovery. Other historical completed outputs and unknown reservations stay. A path that is active, referenced, conflicting, redirected or otherwise uncertain never gains deletion authority from its filename or age. Where locking is unsupported or unavailable, an uncertain reservation is kept, automatic cleanup is off and the conversion fallback does not recover it. There is no general source-library or orphan-file pruning.

## Stop, resume and remove

Run from the same Compose directory and project used for setup:

```sh
docker compose ps
docker compose stop            # stop services, keep containers and data
docker compose start           # resume services
docker compose down            # remove containers and networks, keep data
```

**Destructive:** the following permanently deletes the deployment's named volume, including accounts, progress, thumbnails, converted files and any media copied into it. Confirm your backups and intended project before running it. A separately bind-mounted host media directory is not removed.

```sh
docker compose down --volumes
```

## Backup and restore

This procedure backs up a stopped app's named volume and restores it into a fresh project that uses the same image. It does not cover an upgrade or downgrade, a schema rollback, a live backup or a backup of bind-mounted media. Keep the source volume and the original image so you can roll back, and rehearse on disposable copies before you trust valued data.

### Same-image recovery template

Run these templates in a fresh `sh`-compatible shell and stop on any failed command.

1. In the source Compose project, identify the exact app container, immutable image and named `/data` volume. Do not proceed for a bind mount or an empty volume name:

```sh
set -eu
SOURCE_CONTAINER=$(docker compose ps -q app)
test -n "$SOURCE_CONTAINER"
IMAGE=$(docker inspect --format '{{.Image}}' "$SOURCE_CONTAINER")
SOURCE_TYPE=$(docker inspect --format '{{range .Mounts}}{{if eq .Destination "/data"}}{{.Type}}{{end}}{{end}}' "$SOURCE_CONTAINER")
SOURCE_VOLUME=$(docker inspect --format '{{range .Mounts}}{{if eq .Destination "/data"}}{{.Name}}{{end}}{{end}}' "$SOURCE_CONTAINER")
test "$SOURCE_TYPE" = volume
test -n "$SOURCE_VOLUME"
```

2. Record these identities privately. Stop the app, proxy and every other writer to the volume. The proxy must release its published port before the destination uses the same configuration; stopping only the proxy does not stop database writes:

```sh
docker compose stop proxy app
test "$(docker inspect --format '{{.State.Running}}' "$SOURCE_CONTAINER")" = false
SOURCE_RUNNING_PROXY=$(docker compose ps --status running --quiet proxy)
test -z "$SOURCE_RUNNING_PROXY"
```

3. Set `BACKUP_DIR` to a new absolute directory path outside the data volume, whose parent already exists. Do not create the target directory in advance: the block below creates and verifies it privately, and refuses existing paths or symlinks. These permission checks use GNU `stat` on Linux. The archive contains accounts and sessions; protect it as private data. Archive the entire stopped volume, including SQLite WAL and SHM files if present:

```sh
umask 077
case "$BACKUP_DIR" in /*) ;; *) echo 'BACKUP_DIR must be absolute' >&2; exit 1 ;; esac
test ! -e "$BACKUP_DIR"
test ! -L "$BACKUP_DIR"
mkdir -m 700 -- "$BACKUP_DIR"
test "$(stat -c '%a' "$BACKUP_DIR")" = 700
test "$(stat -c '%u' "$BACKUP_DIR")" = "$(id -u)"
set -C  # refuse to overwrite an existing archive
docker run --rm -i --network none --user 0:0 --entrypoint tar -v "$SOURCE_VOLUME:/data:ro" "$IMAGE" -C /data -cpf - . > "$BACKUP_DIR/data.tar"
test -s "$BACKUP_DIR/data.tar"
test "$(stat -c '%a' "$BACKUP_DIR/data.tar")" = 600
(cd "$BACKUP_DIR" && sha256sum data.tar > data.tar.sha256)
test "$(stat -c '%a' "$BACKUP_DIR/data.tar.sha256")" = 600
```

Require a successful archive command; a nonempty partial archive is not a backup. Keep `data.tar.sha256` beside `data.tar`, with the same private access. Move both together if relocating the backup. The restore block verifies the checksum before extraction.

4. Prepare a **distinct Compose project** with the same immutable app image and data paths. From the directory containing that destination configuration, run `docker compose --project-name frayd-restore create --pull never --no-build`. The command creates containers without starting the app, pulling an image or rebuilding it; the destination configuration must already select the recorded immutable image. This initializes a separate volume with image-owned directory scaffolding. Inspect the destination app's `/data` mount using the same `--project-name frayd-restore`, set `DEST_VOLUME` to its exact name, and require a named volume different from `SOURCE_VOLUME`. Inspect the destination before extraction: it must contain directory scaffolding only, with no existing files. Stop if it has existing data. Never restore over a running or populated volume.

5. Restore into that confirmed empty destination, preserving numeric ownership (the app uses UID and GID 10001). The `-i` flag is required to pass archive bytes on stdin:

```sh
test -n "$DEST_VOLUME"
test "$DEST_VOLUME" != "$SOURCE_VOLUME"
if ! (cd "$BACKUP_DIR" && sha256sum --check data.tar.sha256); then
  echo 'Backup checksum verification failed; nothing was extracted' >&2
  exit 1
fi
docker run --rm -i --network none --user 0:0 --entrypoint tar -v "$DEST_VOLUME:/data" "$IMAGE" -C /data -xpf - < "$BACKUP_DIR/data.tar"
```

6. Start only the destination project with the same image and verify HTTP health, library and media bytes, accounts and saved progress, plus SQLite `PRAGMA integrity_check` and selected database IDs and progress. Keep the original app and proxy stopped while the destination uses their published port. If verification fails, stop the destination app and proxy before restarting the original project, and retain the original volume and image; do not delete the source or retry extraction over populated data. Back up separately mounted media as a separate data source.

These shell checks are safeguards around the steps above. They are not an automated recovery tool and do not guarantee recovery of valued data. Upgrades, downgrades, restores into a legacy schema and rolling back to the original volume are not covered.

## Troubleshooting

- **Page unavailable:** check `docker compose ps` and `docker compose logs --tail=100`. Confirm the loopback URL and chosen published port. The first build downloads images and Rust dependencies and runs tests, Clippy and a release build, so startup can take several minutes. Do not publish the app to solve a local connection failure.
- **Empty library:** confirm the mount's host source exists and UID and GID 10001 can read and traverse it, then choose **Scan library**. A nonempty `FRAYD_LIBRARY` locks Settings; clear it explicitly for database-managed roots. A temporarily unavailable configured root is retained, so check the mount before rescanning.
- **Conversion or thumbnail failure:** check writable space and access in `/data`, including thumbnails and transcodes. Keep the source mount read-only. Media without usable local cover art keeps the symbol fallback.
- **Missing data after recreation:** check the Compose directory and project identity and its volume before creating accounts or rescanning. A different project can select a different volume.
- **A format or URL feature fails:** check the limits below; image tools and browser support differ.

## Supported tools and current limits

- The Docker image includes `ffmpeg` and `ffprobe` for metadata, thumbnails and transcoding. Actual playback still depends on the media format and browser codec support. New-item metadata uses one structured JSON probe for duration and decoded title, synopsis, author and series tags; missing or unusable metadata retains filename fallbacks. Thumbnail seeking reuses that duration; an existing video with no stored duration is probed once when its artwork is missing.
- Media streaming honors a single bounded, open-ended or suffix byte range. Unsatisfiable ranges return 416; HEAD returns full-resource headers with no body. Multipart ranges are ignored and return the full resource; empty files return an empty 200 response.
- CBZ and EPUB handling is built into the server and reads ZIP archives. EPUB container and OPF XML uses the locked `quick-xml` 0.42.0 crate. EPUB metadata is limited to 2 MiB decompressed per entry, chapter and cover assets to 32 MiB, and XML to 100,000 events and a depth of 128. External DTDs and custom entity expansion are not supported, and a failed or oversized cover read does not fall through to an unrelated image. These are per-entry and parser limits, not a total archive or concurrent-memory limit. RAR-based CBR needs the optional `unrar`, which this image does not include.
- Automatic router port mapping needs optional `upnpc`. It is not included, and the image defaults `FRAYD_UPNP=0`. The Compose example supplies neither TLS nor automatic network exposure.
- Online artwork search is not wired in: a filename-first iTunes lookup can match the wrong work. Bringing it back needs verified artist and album matching and an explicit opt-in to the provider. No online provider, environment flag or general catalog metadata lookup is configured. Local tags and filename fallbacks supply ordinary media metadata, and cover extraction follows the local-source rules above.
- YouTube watch parties load the YouTube iframe API in each participant's browser. Each browser needs access to YouTube; the app's isolated Docker network does not block browser-side requests. This feature has not been verified by the [disposable integration checks](development.md#disposable-qa-scope).
- **Add from URL** uses an optional server-side `yt-dlp` executable to download into the library. The supplied image does not include it, and the Compose app has no external routing, so URL ingestion is unavailable in this setup. Native use additionally requires network access and a writable first library root; the read-only media mounts above cannot receive downloads.
- **TV Shows** groups recognized episodes by series and season. Supported video filenames use a complete `S01E02`, `1x02` or whitespace-separated `Season 1 Episode 2` token (case insensitive, at most two season digits and three episode digits); dimensions such as `1920x1080` remain movies. Series metadata takes precedence, then the filename prefix, then up to two parents inside an admitted library root, skipping a numbered Season folder. Rescanning can reclassify an unchanged movie and fill only a NULL episode series without probing or converting media; existing non-NULL values, including an empty string, are preserved. Renaming filenames or changing this metadata is not an episode editor.
- Music, audiobooks and ebooks group by author and named album or series; loose items remain visible beside series folders. Folder tiles do not make media-progress requests.
- Sign-in offers a local email and password, and guest entry. Remember me, Google and SSO sign-in, notifications, and audio Previous and Next buttons are not implemented. Their design fragments are in [docs/design](design/unwired-ui-fragments.html).
- The [native bundle](native-bundle.md) is experimental, with explicit ownership and removal checks. The browser and bootstrap installers are separate and also experimental. On Windows, `tests/test_bootstrap_windows.py` runs `bootstrap.ps1` under Windows PowerShell 5.1 and PowerShell 7 with stub commands and runs the generated startup launcher. It does not install real Git, Rust, FFmpeg or tunnel software. The [Docker quick start](../README.md#run-with-docker) is the startup route this guide documents.
- ZIP readers support stored and deflate archives. Container isolation does not establish that arbitrary media is safe.
- No license file is currently present in the repository. Do not assume redistribution terms from the public repository alone.

### Check a build against upstream

The app makes no upstream requests. Settings links to the upstream GitHub page (sign-in may be required). To check a build, run an explicit operator command from a checkout that already has authenticated Git access:

```sh
python3 scripts/check-upstream.py --build-url http://127.0.0.1:3100
# Alternatively compare a selected source ref, not the running app:
python3 scripts/check-upstream.py --ref HEAD
```

The default remote is the checkout's `upstream` remote, branch `main`. `--remote` selects another existing remote. The command prints JSON with the selected SHA, the upstream SHA and commit time, and the UTC time of the check. With `--build-url` it reads `/api/build` from the running app, and it always fetches into disposable storage. Git operations have 45-second limits within a 90-second total. HTTP uses a 10-second timeout per operation and for reading the body. Prompts and redirects are disabled, and raw errors and remote URLs are not printed. No token is copied or printed, and existing Git credential helpers may be used as normal.

The command changes nothing: no branch, index, working tree, local fetch ref, setting, credential or installed binary is updated. It needs Git and full history, and it runs on the host, outside the app's isolated Docker network.

| Status | Meaning |
|---|---|
| `current` | The revisions are equal. |
| `behind` | The selected revision is an ancestor of upstream. |
| `ahead` | The selected revision contains upstream plus local commits. |
| `diverged` | Neither revision is an ancestor of the other. |
| `unknown` | The check could not finish. The command exits 2, and the `reason` field names the failed check, for example `unknown installed commit`, `shallow or unavailable history` or `upstream fetch failed`. |

Every other status exits 0. These are Git ancestry statements about the moment of the check. They do not say that a dirty build, or a build of unknown state, has upstream's exact bytes.
