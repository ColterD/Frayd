# Operations

[Documentation index](README.md) · [Quick start](../README.md) · [Development and checks](development.md)

These instructions describe the fork draft's Docker setup. Use trusted media on a trusted machine. Guest access and library/media routes are not a private authentication boundary; Compose binds only to loopback and does not supply TLS or public hosting. Guests retain device-scoped playback/reading progress. Saving ebook highlights requires a signed-in account; guest device identifiers are not account credentials.

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

Replace the source with a real directory. The app runs as UID/GID 10001: it needs read access to files and traversal access to directories. Keep the source mount read-only; database, thumbnails and conversions need separate writable storage. Do not solve a permission failure by making the source library writable.

```sh
docker compose config --quiet
docker compose up -d --wait
```

The app sees `/data/library`, not the host path. The image's `FRAYD_LIBRARY=/data/library` overrides the library path saved in Settings. Change the mount source to select a different host folder, then choose **Scan library** after adding or changing media.

Alternatively, copy regular files and directories into a new subdirectory of the default library volume, using additional disk space. Symlinks, including a symlinked source directory, and special files are rejected without following them. Keep the source unchanged throughout the preflight and copy: these steps are not atomic. Run this block in Bash with GNU `find` and GNU `tar`, set the absolute source directory, and choose a new import name using only letters, digits, underscores or hyphens:

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

Use this alternative with the app running and without the read-only library override. GNU tar sets private archive-header modes with owner read/write access and directory traversal; it does not change the source permissions. This also handles sources readable only through group access. Extraction runs as the app's UID/GID 10001 with a private umask, so copied files remain readable and directories traversable by the app without granting group or other access. The new destination must not already exist. A failure can leave a partial import: inspect that directory before scanning or retrying. After a successful copy, choose **Scan library**. Media copied into the named volume shares that volume's deletion and backup lifecycle.

## Configuration and storage

Compose publishes `127.0.0.1:3100`. If that port is occupied, prefix Compose commands with `FRAYD_PORT=3105` and open http://127.0.0.1:3105. This changes the proxy's published port; the app still listens on 8484 inside Docker. Keep the same Compose project name/directory to reuse the same named volume.

| Docker path | Contents |
|---|---|
| `/data/frayd.sqlite` | Accounts, sessions, library metadata and reading/playback progress |
| `/data/library` | Default library, or the read-only host mount above |
| `/data/thumbs` | Generated thumbnails |
| `/data/transcodes/<media-id>/<job-UUID>/` | Converted playback files |

The Compose `data` volume persists database, thumbnails, conversions and any copied media across container recreation. A host library mount has its own storage lifecycle. Runtime services use read-only root filesystems, dropped capabilities, resource limits and temporary `/tmp` storage. Only the proxy publishes a port; the app's internal network has no external routing. See [compose.yaml](../compose.yaml) for exact limits.

Environment overrides are `FRAYD_DB`, `FRAYD_LIBRARY`, `FRAYD_THUMBS`, `FRAYD_TRANSCODES`, `FRAYD_PORT`, `FRAYD_UPNP`, and optional PEM paths `FRAYD_CERT`/`FRAYD_KEY` for native TLS. Container paths must exist inside the container and have the required access. Compose reads the host `FRAYD_PORT` for publishing; other host variables are not automatically passed to the app. The [Dockerfile](../Dockerfile) sets its defaults. Native defaults and checks are in [Development](development.md#native-server).

### Conversion cache limits

Keep generated data writable and outside read-only source media. When `FRAYD_TRANSCODES` is unset, conversions go in `transcodes/` beside the database; Docker explicitly uses `/data/transcodes`.

Unchanged sources retain existing conversion references. A detectable source timestamp or byte-length change on rescan invalidates the current reference, and conversion jobs check actual file and database versions before publication. Legacy references with initially unknown fingerprints remain compatible, but their original source version is unverified. Rescan fingerprints use seconds plus byte length: replacements preserving both values are undetected.

Older completed cache files and ownership records remain until manual cleanup, so cache usage can grow across replacements. Back up data and inspect current playback references before removing historical outputs. Older `.frayd-tmp` files in source libraries are not automatically migrated or removed; inspect and back up legacy files before deciding what to remove. No general cache-pruning procedure is qualified here.

### Interrupted conversions

New conversion reservations record pending, completed and retiring lifecycle
state. On supported Linux locking, a worker and its ffmpeg child hold the job
lease; startup, explicit rescans and the next conversion attempt can reclaim
only proven inactive pending jobs. A cleanup blocked by the database retains
bytes/provenance and retries on one of those later entry points.

Publication marks completion atomically with the playback pointer. Completed
old versions and historical unknown reservations remain retained. Active,
referenced, conflicting, redirected or otherwise uncertain paths never gain
deletion authority from their filename or age. Unsupported/unavailable locking
retains an uncertain reservation instead of enabling automatic cleanup; the
conversion fallback does not qualify recovery on that platform/filesystem.
No general source-library or orphan-file pruning is introduced.

## Stop, resume and remove

Run from the same Compose directory/project used for setup:

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

## Backup, restore and upgrade status

The bounded qualification is a **same-image, stopped-app, named-volume round trip using synthetic data**. It does not qualify an upgrade/downgrade, a schema rollback, live backups or external bind-mounted media backups. Keep the source volume and original image available for rollback. Test recovery against disposable copies before using valued data.

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

2. Record these identities privately. Stop the app and every other writer to the volume. Confirm the app is stopped; stopping only the proxy does not stop database writes:

```sh
docker compose stop app
test "$(docker inspect --format '{{.State.Running}}' "$SOURCE_CONTAINER")" = false
```

3. Set `BACKUP_DIR` to a new absolute directory path outside the data volume, whose parent already exists. Do not create the target directory in advance: the block below creates and verifies it privately, and refuses existing paths or symlinks. These permission checks use Linux/GNU `stat`. The archive contains accounts and sessions; protect it as private data. Archive the entire stopped volume, including SQLite WAL/SHM files if present:

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

4. Prepare a **distinct Compose project** with the same immutable app image and data paths. Create its containers without starting the app (`docker compose … create --pull never` for that project's exact configuration). This initializes a separate volume with image-owned directory scaffolding. Inspect the destination app's `/data` mount as above, set `DEST_VOLUME` to its exact name, and require a named volume different from `SOURCE_VOLUME`. Inspect the destination before extraction: it must contain directory scaffolding only, with no existing files. Stop if it has existing data. Never restore over a running or populated volume.

5. Restore into that confirmed empty destination, preserving numeric ownership (the app uses UID/GID 10001). The `-i` flag is required to pass archive bytes on stdin:

```sh
test -n "$DEST_VOLUME"
test "$DEST_VOLUME" != "$SOURCE_VOLUME"
if ! (cd "$BACKUP_DIR" && sha256sum --check data.tar.sha256); then
  echo 'Backup checksum verification failed; nothing was extracted' >&2
  exit 1
fi
docker run --rm -i --network none --user 0:0 --entrypoint tar -v "$DEST_VOLUME:/data" "$IMAGE" -C /data -xpf - < "$BACKUP_DIR/data.tar"
```

6. Start only the destination project with the same image and verify HTTP health, library/media bytes, accounts and saved progress, plus SQLite `PRAGMA integrity_check` and selected database IDs/progress. If verification fails, stop the destination and retain the original volume/image; do not delete the source or retry extraction over populated data. Back up separately mounted media as a separate data source.

The latest synthetic round trip passed in one uninterrupted invocation on image
`sha256:858879bf50c7050dfb55f93723e8fa74fbde77e4a2447e00eb574d9e45bca0b5`,
built from source `bf3cb589881542cb0f6c4d47d467a9909c7b697d`. It executed the
copyable backup block under a permissive caller umask and verified directory mode
700, archive mode 600, whole-volume copy, numeric ownership, SQLite integrity,
selected media IDs/exact bytes and guest progress. The restored app saved new
progress under UID/GID 10001 and retained it after restart. The lifecycle column
appeared exactly once with its unknown default; no artifact rows were invented.
The updated guide was then requalified in one fresh Docker round trip: the
archive and relative-name checksum sidecar were both mode 600, an intact archive
verified before extraction, and a corrupted copy exited before extraction with
the actual destination still containing only empty image scaffolding.
Accounts and sessions were zero in this fixture, so account/session recovery was
not qualified. Browser/proxy and external bind-media recovery were not exercised.

The earlier `cd880c88` round trip used bounded QA continuations; its historical
receipts remain separate from this later one-shot result. The shell checks above
are safeguards around the tested mechanics, not an automated operator recovery
tool or a guarantee for valued data. Permission checks also refused existing,
symlink and relative targets without modifying them. Version-to-version upgrade,
downgrade, legacy-schema restore and original-volume rollback remain **unqualified**.


## Troubleshooting

- **Page unavailable:** check `docker compose ps` and `docker compose logs --tail=100`. Confirm the loopback URL and chosen published port. The first build downloads images and Rust dependencies and runs tests, Clippy and a release build, so startup can take several minutes. Do not publish the app to solve a local connection failure.
- **Empty library:** confirm the mount's host source exists and UID/GID 10001 can read/traverse it, then choose **Scan library**. Docker's `FRAYD_LIBRARY` overrides Settings.
- **Conversion or thumbnail failure:** check writable space/access in `/data`, including thumbnails and transcodes. Preserve the read-only source mount. Missing online artwork is expected with the app's isolated network.
- **Missing data after recreation:** check the Compose directory/project identity and its volume before creating accounts or rescanning. A different project can select a different volume.
- **A format or URL feature fails:** check the limits below; image tools and browser support differ.

## Supported tools and current limits

- The Docker image includes `ffmpeg` and `ffprobe` for metadata, thumbnails and transcoding. Actual playback still depends on the media format and browser codec support.
- ZIP-based CBZ/EPUB handling is built into the server. RAR-based CBR needs the optional `unrar` executable, which this image does not include.
- Automatic router port mapping needs optional `upnpc`. It is not included, and the image defaults `FRAYD_UPNP=0`. The Compose example supplies neither TLS nor automatic network exposure.
- Outside the isolated Docker network, audio thumbnail fallback can send filename-derived search terms to Apple's iTunes Search API and download artwork. Local metadata/embedded artwork extraction runs first. The supplied Compose app has no external routing, so online artwork lookup is unavailable and a missing thumbnail is expected.
- YouTube watch parties load the YouTube iframe API in each participant's browser. Each browser needs access to YouTube; the app's isolated Docker network does not block browser-side requests. This feature has not been verified by the [disposable integration checks](development.md#disposable-qa-scope).
- **Add from URL** uses an optional server-side `yt-dlp` executable to download into the library. The supplied image does not include it, and the Compose app has no external routing, so URL ingestion is unavailable in this setup. Native use additionally requires network access and a writable library; the read-only media mount above cannot receive downloads.
- Videos are currently classified as movies; season/episode organization is not implemented.
- The browser installer pages and bootstrap scripts are experimental; the [Docker quick start](../README.md#run-with-docker) is the reproducible startup route documented here.
- ZIP readers support stored and deflate archives. Container isolation does not establish that arbitrary media is safe.
- No license file is currently present in the repository. Do not assume redistribution terms from the public repository alone.
