# Development and verification

[Documentation index](README.md) · [Quick start](../README.md) · [Operations](operations.md) · [Native bundle](native-bundle.md)

Run these commands in an isolated checkout, with disposable data. Passing them does not authorize rollout or public exposure.

## Native server

Install a current Rust toolchain, Clippy and a C compiler. On Linux, the Rust tests also need `ffmpeg` and `ffprobe` on `PATH` for the replacement-duration check. The Docker build stage supplies both tools before it runs the tests. Keep `client/` beside `server/`. To run a packaged install instead, follow the [native bundle guide](native-bundle.md). From `server/`:

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
FRAYD_BIND=127.0.0.1 FRAYD_LIBRARY=/absolute/path/to/your/media cargo run --locked
```

Use a disposable media directory. A native run has no Compose network isolation, so set `FRAYD_BIND=127.0.0.1` to keep it on loopback. The default port is 8484.

The default database is `frayd.db` under the server source directory that was compiled in, and thumbnails and conversions default to the same place. Moving a bare server binary does not relocate them. Leave `FRAYD_TRUSTED_PROXY` unset for a direct native run, so login rate limits use the connection address. Set `FRAYD_DB`, `FRAYD_THUMBS` and `FRAYD_TRANSCODES` to writable disposable paths, separate from any read-only source library, and set `FRAYD_CLIENT_DIR` to the client assets you intend to serve. The packaged launcher sets all of these to private paths and binds to loopback.

See [configuration and storage](operations.md#configuration-and-storage) for the environment overrides and cache limits. Install `ffmpeg` and `ffprobe` yourself for native runs. The Docker image includes them.

## Canonical checks

From the repository root, with Node.js 22 or later, Python 3.11 or later, Docker Engine and Compose:

Run Python qualification commands with optimization disabled: do not pass `-O` or `-OO`, and leave `PYTHONOPTIMIZE` unset. The integration, build-context, DNS-recovery and download-qualification scripts refuse optimized execution; keep the unittest suites unoptimized because some checks use plain `assert`.

On macOS, the full Python test command below requires Python 3.13 or later for the native harness's `os.waitid` child-custody checks. See [native checks](native-bundle.md#checks). Other platforms retain the Python 3.11 minimum.

```sh
node --check client/js/app.js
node --check client/js/party.js
node --check client/js/build-info.js
node --test tests/*.test.cjs
python3 -m unittest discover -s tests -p 'test_*.py'
docker compose config --quiet
git diff --check
```

CI uses Node.js 22.23.4 on its Linux x64 runner. It reuses an exact valid toolcache entry or downloads the official archive, checks its pinned SHA-256 in temporary job storage, and verifies the executable version before adding it to `PATH`. To update the CI version, check the official release and its archive checksum together. Local checks need Node.js 22 or later with the global WebSocket API.

For the Linux x86_64 hash-locked lint environment, create a temporary directory and keep its path for cleanup:

```sh
FRAYD_LINT_DIR=$(mktemp -d)
python3 -m venv "$FRAYD_LINT_DIR/venv"
"$FRAYD_LINT_DIR/venv/bin/python" -m pip install --disable-pip-version-check --require-hashes --no-deps --only-binary=:all: -r .github/requirements-ci.txt
"$FRAYD_LINT_DIR/venv/bin/ruff" check --no-cache --select E4,E7,E9,F,B905 scripts tests
```

Remove only that temporary directory when you finish. The locked wheel targets Linux x86_64, so other platforms need their own lint setup.

Build the QA images before the context and integration checks:

```sh
docker build -t frayd-qa:local .
docker build -f Dockerfile.proxy -t frayd-proxy-qa:local .
docker run --rm --network none --read-only --tmpfs /tmp --entrypoint nginx frayd-proxy-qa:local -t
python3 scripts/check-build-context.py
python3 scripts/integration.py
python3 tests/test_dns_recovery.py
python3 scripts/audit-dependencies.py
```

The Debian stages in the [Dockerfile](../Dockerfile) are pinned. Each `apt-get update` has a 180-second limit and each install a 360-second limit, with a ten-second kill grace. APT uses 30-second HTTP and HTTPS timeouts, two retries and strict update errors, so package acquisition fails within a known time. The [offline package tests](../tests/test_build_packages.py) simulate a stalled download and a partial package index during Python test discovery.

### What CI runs

The [Dockerfile](../Dockerfile) runs the locked Rust tests, Clippy with warnings denied and the release build. [Checks CI](../.github/workflows/checks.yml) runs:

- client syntax checks and tests
- the real browser DOM regression
- locked Python lint
- Compose validation
- both image builds
- the context, integration and controlled DNS-failure checks
- the Python tests
- the live dependency audit

On Linux, CI also exports both native binaries and the compiler metadata from the Docker build, then runs the native ownership and runtime suite with `FRAYD_NATIVE_REQUIRE_QUALIFICATION=1`, so a missing input fails the job. The Python discovery in the dependency-audit job runs without those inputs, so it checks packaging and static policy and skips the helper and real-server scenarios. The inputs are listed under [native checks](native-bundle.md#checks).

The Cargo commands in [Native server](#native-server), `git diff --check` and the standalone nginx probe are extra local checks that the workflow does not run on their own. [CodeQL](../.github/workflows/codeql.yml) analyzes the source and the workflows separately.

The separate `windows-native-build` job uses `windows-2025`, official Rust 1.99.1 and two Cargo build jobs. It checks:

- A locked release build of `frayd-server.exe` and `frayd-native.exe` through the source-metadata wrapper.
- The server's exact clean source commit and package version, and the helper's unsupported-platform refusal with exit code 2.
- Loopback health, matching HTTP build identity and all eight public asset bytes, using an empty synthetic library and a fresh private database. Downloads and UPnP stay disabled.
- Cleanup of the child through its retained process handle. After child exit, a bounded startup-log check requires zero errors and panics, and reports warnings separately.

This covers basic source builds and startup. The separate `windows-bootstrap` job uses fixtures and stubbed Cargo. Neither job qualifies a full Windows installation, browser playback, media tools or production deployment. The native bundle helper supports Linux and macOS only. A configured job is not proof of a pass; Windows qualification requires a passing run at the exact source revision.

The bootstrap regressions in `tests/test_bootstrap_windows.py` run `bootstrap.ps1` under Windows PowerShell 5.1 and PowerShell 7 with stub commands and run the generated startup launcher. They do not install real Git, Rust, FFmpeg or tunnel software. Generated-command tests use both parent engines; the generated command always starts a Windows PowerShell 5.1 child, including from a PowerShell 7 parent. The Windows PowerShell 5.1 policy control requires direct file execution under `Restricted` to fail and the generated child under `Bypass` to succeed. These controls qualify observed behavior; they do not certify that Group Policy is absent. The tests bound the owned process tree and use a disposable harmless child to check entry-file cleanup. Required Windows qualification runs both parent shells and rejects missing engines or skipped cases.

The manual [macOS workflow](../.github/workflows/release-mac.yml) builds the native bundle on `macos-15` (ARM64) and `macos-15-intel` (x86_64), with a 35-minute limit per job. It checks the actual architecture and runs each platform's native lifecycle suite. It has no tag trigger, publishes nothing and uploads no artifact.

## Disposable QA scope

The integration script defaults to the `frayd-qa:local` and `frayd-proxy-qa:local` images. `--app-image` and `--proxy-image` override them. It starts a uniquely named disposable project on a random loopback port with synthetic media, and it checks:

- first-owner setup, scans, and comic and EPUB replacement
- bounded and suffix Range bytes, unsatisfiable ranges and HEAD responses
- saved progress, container recreation and logout
- proxy recovery after the app's IP address changes
- responsive read paths during a blocked scan
- codec conversion into writable storage while sources stay read-only
- that a replaced source produces fresh converted bytes, not stale cache output, and keeps its IDs and progress

The Docker call helper gives each call a 120-second deadline by default, and a call can pass its own. The `finally` path attempts to remove its own containers, networks and volume. If cleanup also fails, it reports that error and keeps the original scenario failure. The resource-inventory assertions run only after a successful scenario; a failed scenario does not prove that no resources remain. It never targets the normal `frayd` project. Keep QA tags and data separate from your deployment.

The Docker context is an exact allowlist in `.dockerignore`: the Dockerfiles, `nginx.conf`, the server sources, the client files the images use and the single generated `tests/fixtures/native-resume.mp4` needed by Rust test compilation, with denials for databases, `.env` files, editor backups and OS files. The Docker and macOS isolated builds copy only that fixture alongside the server sources; it does not enter runtime images or native packages. `scripts/check-build-context.py` exports a synthetic scratch context to check the exact fixture allowlist and reject neighboring tests and secrets. It also uses synthetic client files and plants `.env`, database, notes and editor-backup sentinels beside the normal asset. It then checks that the exported client tree holds only `client/js/app.js`, and that no sentinel or installer text reaches the image.

These scenarios are automated HTTP and storage checks. They do not cover browser playback, every media format, watch-party behavior, backup restoration, upgrades or public-hosting security. Client tests that run in a VM use synthetic browser and network boundaries, so real browser sandbox behavior needs a real browser.

### Docker download checks

CI is configured to require three separate modes after building the QA images above. Use an existing Chrome or Chromium browser; set `BROWSER_BIN` when it is not on `PATH`. Set `FRAYD_EXTERNAL_FIXTURE_URL` to the public raw GitHub URL of `tests/fixtures/native-resume.mp4` at a full commit matching the checkout. CI derives the URL from the event's head repository and revision and passes it as a quoted environment argument.

```sh
python3 scripts/qualify-downloads.py
python3 scripts/qualify-downloads.py --external-tls --external-url "$FRAYD_EXTERNAL_FIXTURE_URL"
python3 scripts/qualify-downloads.py --private-assets
```

- The controlled mode uses an owned nginx HTTP origin for actual MP4 download, indexing, streaming, FFmpeg decode and muted browser playback. It covers authentication and read-only settings, invalid/HTML/non-MP4/audio-only responses, failed requests, collisions, and rollback on the same image and volume. Disabled, missing-native-tool and unknown-backend controls check refusal without job or library effects. A healthy egress-only fixture is reachable from the proxy and refused by the default app.
- The external mode submits one Rust HTTPS ingestion request, waits for indexing and byte-compares its stream with the checkout's exact Git fixture. It bounds the local blob to 1 MiB and the stream read to the expected size plus one byte. It supplies no external credentials and never retries the ingestion POST. Success qualifies that endpoint, not general site compatibility.
- The private-assets mode builds only a tiny layer from the prebuilt app image, with public source directories at 0700 and files at 0600. It reuses the production normalization step and checks all eight HTTP responses against source bytes, root ownership, runtime directories at 0755 and files at 0644. It rebuilds no Rust code and removes its owned image aliases afterward.

Keep each mode's exit status, image/source identities, platform and cleanup result. Configured CI checks are not passing receipts, and passing one mode does not satisfy the others. The qualifiers use disposable projects; rollback keeps the volume, while final fixture cleanup removes only those projects' resources. These checks do not authorize deployment. See [opt-in operation and rollback](operations.md#opt-in-docker-downloads) for the operator commands.

## Dependency audit policy

The audit queries OSV for the exact versions in `server/Cargo.lock`, prints findings and exits nonzero on applicable advisories. A network or API failure, or incomplete pagination, also fails it.

One database disagreement is handled explicitly. OSV flags `block-buffer 0.10.4` for `GHSA-qwgh-2vcv-g2f7`, while the publisher lists only `>=0.11.0,<0.12.1`. Each audit fetches the publisher advisory from the RustCrypto GitHub API. It classifies the finding as `not_affected` only when the advisory identity, package and range match exactly. Changed, malformed or unavailable publisher evidence keeps the finding fatal, and so does every other finding. CI tests this policy and runs the live query separately. See the [audit implementation](../scripts/audit-dependencies.py) and its [policy tests](../tests/test_audit_dependencies.py).

## Browser checks

The automated suite does not replace playing your own media in a real browser.

### Real DOM regression

With Node.js 22 or later (the global WebSocket API is required) and a Chrome or Chromium executable on `PATH`, run:

```sh
node tests/browser-regression.cjs
```

Set `BROWSER_BIN` to the executable when it is not on `PATH`. The runner downloads no browser and adds no npm package. It serves a loopback-only fixture and uses a new temporary browser profile. It deletes that profile only after it confirms, with bounded waits, that the browser's process group has stopped. If shutdown fails, the runner keeps the profile and reports the cleanup error, separately from any test failure.

The runner prints its result as JSON and also writes it to `result.json` in `FRAYD_DOM_RESULT_DIR`, or by default in a new private `frayd-dom-regression-*` directory under the system temporary directory. The result includes the browser version, the executable hash and the hashes of the tested sources. CI uses its runner's browser and fails when none exists.

The suite runs the production client, the generated sandboxed EPUB frame and vendored Plyr in that browser. The fixture's API handlers control annotation responses and redirect Plyr's icon URL to the local sprite. Media IDs 9, 10 and 11 use synthetic player events. Native IDs 20, 21 and 22 use a short H.264 fixture that the repository owns. Six controls on that fixture require `readyState` 4, a video width of 160 and a positive decoded-frame count. It does not cover audible audio, other codecs or timing across separate clients. Progress-save controls require the current tracker, metadata and a matching stream, including a negative check against stale local audio bookmarks.

The browser comes from your operating system or CI image, so keep it updated. Cargo audits do not scan it.

### Dashboard and artwork

The seven dashboard shelves share `categoryShelf()` in `client/js/app.js`. They show the newest items first by default, with one oldest-first toggle per category. A folder sorts by the maximum or minimum `created_at` of its members. TV episode order and the render-scoped progress reader are separate. `artwork()` uses the thumbnail API, with a URL version taken from the encoded `artwork_version` field and never a cache path. The library and search API returns that nullable opaque basename only for the same media ID's canonical UUID file or retained legacy name. Missing art falls back to a category symbol. An artwork image does not establish that the title or artist matches.

Scanner tests in `server/src/scanner.rs` cover seven local-source kinds, missing-cover repair, guarded publication, replacement and conservative retirement. Tests in `server/src/main.rs` cover thumbnail MIME types and deletion. The integration test reads the UUID JPEG path selected by the database and asserts exact bytes, rollback preservation and 204 cleanup. To check dashboard sort, representative folder art and the 320×568 footer reachability, run the Rust tests and strict Clippy from [Native server](#native-server), and run the Node and real-DOM commands from [Canonical checks](#canonical-checks). The EPUB DOM controls check publication of a fresh frame and rejection of retired messages.

No online artwork provider is configured. The [supported tools](operations.md#supported-tools-and-current-limits) list explains why.

### EPUB bounds and reader layout

The EPUB container and OPF path uses `quick-xml` 0.42.0 with default features off, locked in `server/Cargo.lock`. It was chosen for namespace-aware, bounded parsing.

The size and parser limits are listed under [supported tools](operations.md#supported-tools-and-current-limits). The reader takes at most the limit plus one byte from the stream instead of trusting ZIP headers. XML parsing ignores external DTDs and does not expand custom entities. URI matching decodes once and keeps literal percent names. A rejected entry stops the cover fallback before FFmpeg runs or output is published.

The browser reader waits for fonts and a positive layout before it reports a page or saves a resume position. A resize can wake a document that could not be measured. A chapter response that is rejected shows a visible unavailable state and saves nothing. Closing the reader also needs the current ready document, generation and frame, so a pending or failed chapter cannot replace a saved bookmark. To check this, run the Rust, Clippy and audit commands, `node --test tests/epub-message.test.cjs` and the real-DOM regression.

### Unwired UI design fragments

[docs/design/unwired-ui-fragments.html](design/unwired-ui-fragments.html) holds the markup for five controls that Frayd does not implement. The shipped UI contains none of them. Nothing serves or loads the fragments page. Each fragment is a `<template>` whose controls are disabled, have no IDs and no event handlers, with a comment naming where it belonged. They are design notes, not features.

| Fragment | Contract to define before activation |
| --- | --- |
| `unwired-remember` | Token and identity storage, session expiry, the unchecked behavior, logout and a shared-device policy. The checkbox must change that policy. |
| `unwired-social-auth` (Google and SSO) | Configured provider routes, redirect and state validation, errors, and safe account linking with the local household account model. |
| `unwired-notifications` | An authorized event source, a real notification surface, empty and error states, and read and dismiss persistence. |
| `unwired-hero-info` | A details view for the selected media, with close and focus behavior and accurate metadata. The hero play action must keep working. |
| `unwired-audio-navigation` (Previous and Next) | Queue order and boundaries, previous-track versus restart behavior, per-track resume and progress ownership, and host and guest authority in a party. |

To activate a fragment, implement its contract and an accessible interaction, add tests for the paths it touches, move the control into the live UI, and check the workflow in a real browser. Git history holds the original markup and styling.

`python3 -m unittest discover -s tests -p 'test_unwired_ui.py'` asserts that element IDs in `client/index.html` are unique, that every fragment has a row in the table above, and that the fragments stay inert: their controls are disabled, have no IDs or event handlers, and `client/index.html` does not link the fragments page. The real DOM suite checks the default auth and audio controls.

### Party protocol

`POST /api/party` returns a public code and a private `host_capability`. Host socket registration requires that proof and the creator's identity in the current session generation. The client offers `frayd-party` and `frayd-host.<proof>` as WebSocket protocols, and the server selects only `frayd-party`. Never put the proof in URLs, storage, public messages or logs. Ordinary guest joins carry no proof.

Party messages show the public labels `Host` and `Guest N`, and carry a separate `self_label` for the current client's UI. Membership and authorization identity stay separate from those labels.

The preflight check and the post-upgrade registration share one role predicate and run before any membership change. The tests cover first-slot and connected-host impersonation, malformed, duplicate and stale offers, a wrong identity, and legitimate reconnects. Any guest can still create or replace a party in a trusted household. The proof authorizes host controls in an existing room. It does not reserve party creation.

A live guest key cannot register again until its earlier outbound receiver closes or cleanup removes it. Preflight and registration both apply this check, and stale cleanup cannot remove a permitted successor. An immediate rejoin with the same key can be refused until server retirement finishes. This protects active membership. It does not authenticate first registration or inactive device identities, so public device labels are local identifiers you trust. Host replacement with the valid creator proof is still supported.

After you upgrade the client and server, refresh open tabs before you host a new party. Mixed client and server versions are untested.

### Legacy comic page names

Page labels persisted by ZIP 0.6.6 can differ from ZIP 9 labels even when the archive bytes are unchanged. The comic reader compares the entire cached list with the unique current names and the historical, flag-aware names. It keeps the cached order and reads the selected physical entry. It never rewrites the cache or user progress. An ambiguous match is refused, and so is a match against an archive that reports duplicate central-directory names. A canonical subset still uses exact lookup, and the directory, RAR and EPUB readers keep their existing paths.

The decoding policy follows the zip 0.6.6 crate. The CP437 high half is standard encoding data generated with Python's `cp437` codec. The server adds no dependency and no second ZIP parser. Tests with a real persisted cache over TCP and real archives cover old names, order, progress, canonical subsets, flag handling, collisions and refusal.

### Build identity

`Cargo.toml` owns the package version. `GET /api/build` (`Cache-Control: no-store`) and `frayd-server --build-info` return the same compile-time JSON: `version`, the full `commit` or null, an optional `commits_since_tag`, and `dirty` as true, false or null. The CLI exits before it opens the database. The sign-in screen, sidebar and Settings show the version and short commit, and the tooltip carries the full identity. A dirty build is marked as dirty, and a build with no source information is marked unknown. A commit identifies a base, not the exact bytes of a dirty build.

Capture the identity before you isolate the source, and use the same capture for every output. The standard local routes are:

```sh
python3 scripts/build-metadata.py -- cargo build --locked --manifest-path server/Cargo.toml --release
python3 scripts/build-metadata.py -- docker build -t frayd:local .
python3 scripts/build-metadata.py -- docker compose build
```

The wrapper supplies `FRAYD_BUILD_COMMIT`, `FRAYD_BUILD_COMMITS_SINCE_TAG` and `FRAYD_BUILD_DIRTY`. `--env` prints only those nonsecret values, which CI writes to an environment file before it copies the source for the macOS build. Docker accepts the three values as build arguments and Compose forwards them to the builder. A plain Docker or Compose build without the wrapper reports an unknown identity, because `.git` stays out of the context. A plain Cargo build in a checkout reads Git itself, while an isolated build must be given the values. Each Git probe has a 15-second deadline. A timeout or unavailable Git command leaves that field unknown. Missing or malformed fields read as unknown. Never derive the identity from runtime environment variables.

The Linux export uses the same Docker arguments as the image. Native packaging asks both binaries for their compile-time identity. It refuses identical binary bytes, a helper without the `frayd-native` role, or malformed or mismatched helper version, commit, dirty state or tag distance. The server base commit and package version must match the packaging checkout. The hashed `build-info.json` records the binary's identity, and `manifest.json` records the packaging checkout under `source`. This assumes a native binary running on the host it was built for. It does not cover cross-built binaries, and it does not attest build arguments that a caller supplied.
