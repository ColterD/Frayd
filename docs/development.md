# Development and verification

[Documentation index](README.md) · [Quick start](../README.md) · [Operations](operations.md) · [Historical verification ledger](verification/coderabbit-follow-up.md)

Use an isolated checkout of the exact commit being qualified and disposable data. These are repeatable commands derived from the checked-in source and workflow, not a claim that a new checkout has passed. The fork draft remains separate from deployment; test success does not authorize rollout or public exposure.

## Native server

Install a current Rust toolchain, Clippy and a C compiler. Keep `client/` beside `server/`: the server embeds that source path at build time. From `server/`:

```sh
cargo test --locked
cargo clippy --locked --all-targets
cargo build --locked --release
FRAYD_LIBRARY=/absolute/path/to/your/media cargo run --locked
```

Use a disposable media directory for development. Native runs have no Compose network isolation. The default port is 8484; the default database is `frayd.db` under the build-time server source directory. Thumbnails and conversions default beside the database. Configure `FRAYD_DB`, `FRAYD_THUMBS` and `FRAYD_TRANSCODES` for disposable writable data, separate from any read-only source library. See [configuration and storage](operations.md#configuration-and-storage) for environment overrides and cache caveats. Native media tools such as `ffmpeg`/`ffprobe` must be available separately; the Docker image supplies those tools.

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

CI selects its preinstalled Node.js 22.23.4 x64 toolcache entry and verifies the exact executable version before adding it to the job PATH. A missing or changed cache entry fails the job; updating this CI version requires checking the runner image and the official Node release. Local checks support Node.js 22+ with the global WebSocket API.

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

The [Dockerfile](../Dockerfile) runs locked Rust tests, Clippy and the release build. [Checks CI](../.github/workflows/checks.yml) runs client syntax/tests, real browser DOM regressions, locked Python lint, Compose validation, both image builds, context/integration and controlled DNS-failure checks, Python tests and the live dependency audit. Native Cargo commands, `git diff --check` and the standalone nginx configuration probe are additional local checks; the workflow does not invoke them separately. [CodeQL](../.github/workflows/codeql.yml) provides separate source/workflow analysis. Remote reviews, coverage and check conclusions must be read for the actual PR head; this guide does not reproduce their hosted results.

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
loopback-only fixture, creates a new temporary browser profile and removes that
profile after the run, after stopping its owned process group with bounded waits.
Runner and cleanup failures remain separate in the result. It executes the production client, generated sandboxed
EPUB frame and vendored Plyr. Fixture APIs control annotation responses; player
events are synthetic and do not establish decoded playback or physical audio.
The fixture redirects Plyr's icon URL to the existing local sprite.

The output records the actual browser version, executable hash and tested source
hashes. CI uses its runner-provided browser and fails if no browser is available;
this is a changing test environment, not an immutable browser qualification.
Keep the browser supplied by your operating system or CI image updated. No new
npm dependency is introduced, and the Cargo audit does not scan that external
browser. These DOM checks complement the manual media matrix; they do not close
its held interactive acceptance cases.
