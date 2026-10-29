# Development and verification

[Documentation index](README.md) · [Quick start](../README.md) · [Operations](operations.md) · [Historical verification ledger](verification/coderabbit-follow-up.md)

Use an isolated checkout of the exact commit being qualified and disposable data. These are repeatable commands derived from the checked-in source and workflow, not a claim that a new checkout has passed. The fork draft remains separate from deployment; test success does not authorize rollout or public exposure.

## Native server

Install a current Rust toolchain, Clippy and a C compiler. On Linux, the Rust tests also require `ffmpeg` and `ffprobe` on `PATH` for the real replacement-duration oracle. The Docker build stage supplies both tools before running tests. Keep `client/` beside `server/` for the source workflow. For a packaged installation, follow the [native bundle guide](verification/native-bundle.md). From `server/`:

```sh
cargo test --locked
cargo clippy --locked --all-targets
cargo build --locked --release
FRAYD_LIBRARY=/absolute/path/to/your/media cargo run --locked
```

Use a disposable media directory for development. Native runs have no Compose network isolation. Set `FRAYD_BIND=127.0.0.1` for loopback access. The default port is 8484; the default database is `frayd.db` under the build-time server source directory. Thumbnails and conversions default beside the database. Configure `FRAYD_DB`, `FRAYD_THUMBS` and `FRAYD_TRANSCODES` for disposable writable data, separate from any read-only source library; set `FRAYD_CLIENT_DIR` to the intended client assets. Raw `cargo run` retains its compile-time database default: moving a bare server binary does not qualify relocation. The packaged launcher supplies explicit private data/client paths and loopback binding, and verifies ownership before start/removal. `default-run = "frayd-server"` preserves `cargo run` after adding the `frayd-native` helper binary; the helper adds no Cargo dependency. See [configuration and storage](operations.md#configuration-and-storage) for environment overrides and cache caveats. Native media tools such as `ffmpeg`/`ffprobe` must be available separately; the Docker image supplies those tools.

## Canonical checks

From the repository root, with Node.js 22+, Python 3.11+, Docker Engine and Compose:

```sh
node --check client/js/app.js
node --check client/js/party.js
node --test tests/*.test.cjs
python3 -m unittest discover -s tests -p 'test_*.py'
docker compose config --quiet
git diff --check
```

CI requires Node.js 22.23.4 on its Linux x64 runner. It reuses an exact valid toolcache entry or downloads the official archive with a pinned SHA-256 into temporary job storage, then verifies the executable version before adding it to PATH. Updating the CI version requires checking the official release and archive checksum together. Local checks support Node.js 22+ with the global WebSocket API.

For the Linux x86_64 hash-locked lint environment, create a temporary directory and retain its path for cleanup:

```sh
FRAYD_LINT_DIR=$(mktemp -d)
python3 -m venv "$FRAYD_LINT_DIR/venv"
"$FRAYD_LINT_DIR/venv/bin/python" -m pip install --disable-pip-version-check --require-hashes --no-deps --only-binary=:all: -r .github/requirements-ci.txt
"$FRAYD_LINT_DIR/venv/bin/ruff" check --no-cache --select E4,E7,E9,F,B905 scripts tests
```

Remove only that created temporary directory when finished. The locked wheel targets Linux x86_64; other platforms need separately qualified tooling. No new application dependency is introduced by these checks.

Build the neutral QA images before running the context and integration checks:

```sh
docker build -t frayd-qa:local .
docker build -f Dockerfile.proxy -t frayd-proxy-qa:local .
docker run --rm --network none --read-only --tmpfs /tmp --entrypoint nginx frayd-proxy-qa:local -t
python3 scripts/check-build-context.py
python3 scripts/integration.py
python3 tests/test_dns_recovery.py
python3 scripts/audit-dependencies.py
```

The [Dockerfile](../Dockerfile) runs locked Rust tests, Clippy and the release build. [Checks CI](../.github/workflows/checks.yml) runs client syntax/tests, real browser DOM regressions, locked Python lint, Compose validation, both image builds, context/integration and controlled DNS-failure checks, Python tests and the live dependency audit. Linux CI also exports both native binaries and actual compiler metadata from the Docker build and runs the native ownership/runtime suite with `FRAYD_NATIVE_REQUIRE_QUALIFICATION=1`; missing inputs fail instead of silently skipping. Default Python discovery in the dependency-audit job still checks packaging/static policy and explicitly skips helper or actual-server scenarios when their inputs are absent. See [native reproducible checks](verification/native-bundle.md#reproducible-checks) for the required inputs. Native Cargo commands, `git diff --check` and the standalone nginx configuration probe are additional local checks; the workflow does not invoke them separately. [CodeQL](../.github/workflows/codeql.yml) provides separate source/workflow analysis. Remote reviews, coverage and check conclusions must be read for the actual PR head; this guide does not reproduce their hosted results.

The manual [macOS qualification workflow](../.github/workflows/release-mac.yml) uses standard `macos-15` ARM64 and `macos-15-intel` Intel runners, each bounded to 35 minutes. It checks actual architecture and executes each platform's native lifecycle suite. It has no tag trigger, publication or artifact upload, and introduces no new action. The proposed upload action remains rejected pending a qualified dependency check. Actual Mac execution is held until the coordinator accepts the final source and local gates; workflow configuration is not a Mac pass. Keep final source/image and architecture results in the [PR description](https://github.com/ColterD/Frayd/pull/2).

## Disposable QA scope

The integration script defaults to `frayd-qa:local` and `frayd-proxy-qa:local`; `--app-image` and `--proxy-image` override them. It creates a unique disposable project, random loopback port and synthetic media. It checks first-owner setup, scans, comic/EPUB replacement, Range bytes, saved progress, recreation and logout. It also checks proxy recovery after an app IP change, responsive read paths during a blocked scan, and actual codec conversion into writable storage while sources remain read-only. Source replacement checks distinguish fresh converted bytes from stale cache output while retaining IDs and progress.

The context check builds with synthetic client/environment/installer sentinels to verify exclusions. The integration script removes its own containers, networks and volume, preserves the original failure if cleanup also fails, and checks for remaining project resources. It does not target the normal `frayd` project. Keep QA tags and data separate from deployment.

These are automated HTTP/storage scenarios. They do not prove browser playback, every media format, watch-party behavior, backup restoration, upgrades or public-hosting security. Client VM tests use synthetic browser/network boundaries; actual browser sandbox behavior needs browser verification. Detailed historical scenarios and immutable source/image bindings remain in the [ledger](verification/coderabbit-follow-up.md).

## Dependency audit policy

The audit queries OSV for exact versions in `server/Cargo.lock`, prints findings and exits nonzero on applicable advisories. Network/API failures and incomplete pagination also fail the command.

One narrow database disagreement is handled explicitly: OSV flags `block-buffer 0.10.4` for `GHSA-qwgh-2vcv-g2f7`, while the publisher lists only `>=0.11.0,<0.12.1`. Each audit fetches the publisher advisory from the RustCrypto GitHub API and requires that exact advisory identity, package and range before classifying this finding as `not_affected`. Changed, malformed or unavailable publisher evidence keeps it fatal. All other findings remain fatal. CI tests this policy and runs the live query separately; historical clean results do not predict future advisories. See [audit implementation](../scripts/audit-dependencies.py) and [policy tests](../tests/test_audit_dependencies.py).

## Browser acceptance

The automated suite does not substitute for the [manual browser acceptance matrix](verification/browser-acceptance.md). That page records its exact media, image/source identities, observed failures and corrected-image acceptance status separately from historical integration results.

### Real DOM regression

With Node.js 22+ (the global WebSocket API is required) and an existing Chrome/Chromium executable on PATH, run:

```sh
node tests/browser-regression.cjs
```

Set `BROWSER_BIN` to an existing executable when it is not discoverable on PATH.
The runner performs no browser download and adds no npm package. It serves a
loopback-only fixture and creates a new temporary browser profile. It removes that
profile only after confirming its owned process group has stopped with bounded waits.
If shutdown fails, the runner retains the profile and reports the cleanup error.
Runner and cleanup failures remain separate in the result. It executes the production client, generated sandboxed
EPUB frame and vendored Plyr. Fixture APIs control annotation responses.
Media IDs 9/10/11 use synthetic player events. Native IDs 20/21/22 use the short
owned H.264 fixture; six controls require readyState 4, video width 160 and a
positive decoded-frame count. That fixture does not qualify physical audio,
other codecs or distinct-client timing. Separate real-player observations are
recorded in the [browser acceptance checkpoints](verification/browser-acceptance.md).
Native progress-save controls also require the current tracker, metadata and
matching stream, including a negative check for stale local audio bookmarks.
The fixture redirects Plyr's icon URL to the existing local sprite.

The output records the actual browser version, executable hash and tested source
hashes. CI uses its runner-provided browser and fails if no browser is available;
this is a changing test environment, not an immutable browser qualification.
Keep the browser supplied by your operating system or CI image updated. No new
npm dependency is introduced, and the Cargo audit does not scan that external
browser. These DOM checks complement the manual media matrix; they do not close
its held interactive acceptance cases.

### Party protocol

`POST /api/party` returns a public code and private `host_capability`. Host socket
registration requires that proof plus the creator identity in the current
session generation. The client offers `frayd-party` and `frayd-host.<proof>` as
WebSocket protocols; the server selects only `frayd-party`. Do not put the proof
in URLs, storage, public messages or logs. Ordinary guest joins carry no proof.

Preflight and post-upgrade registration share the same role predicate before
membership changes. Tests cover first-slot and connected-host impersonation,
malformed/duplicate/stale offers, wrong identity and legitimate reconnects.
Guest creation/replacement remains public for the trusted household; the proof
authorizes existing-room host controls, not exclusive global creation rights.
After a client/server upgrade, refresh open tabs before hosting a new party.

A live guest key cannot be registered again until its prior outbound receiver
closes or cleanup removes it. Both preflight and registration use this occupancy
check; stale cleanup cannot remove a permitted successor. An immediate same-key
rejoin can be rejected until server retirement completes. This protects active
membership, not first-registration or inactive device-identity authentication;
public device labels remain trusted-local identifiers. Valid creator-proof host
replacement stays supported.
