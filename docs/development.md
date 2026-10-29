# Development and verification

[Documentation index](README.md) · [Quick start](../README.md) · [Operations](operations.md) · [Historical verification ledger](verification/coderabbit-follow-up.md)

Use an isolated checkout of the exact commit being qualified and disposable data. These are repeatable commands derived from the checked-in source and workflow, not a claim that a new checkout has passed. The fork draft remains separate from deployment; test success does not authorize rollout or public exposure.

## Native server

Install a current Rust toolchain, Clippy and a C compiler. On Linux, the Rust tests also require `ffmpeg` and `ffprobe` on `PATH` for the real replacement-duration oracle. The Docker build stage supplies both tools before running tests. Keep `client/` beside `server/` for the source workflow. For a packaged installation, follow the [native bundle guide](verification/native-bundle.md). From `server/`:

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
FRAYD_BIND=127.0.0.1 FRAYD_LIBRARY=/absolute/path/to/your/media cargo run --locked
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

The pinned Debian stages in the [Dockerfile](../Dockerfile) bound package update
to 180 seconds and install to 360 seconds, each with a ten-second kill grace.
APT uses 30-second HTTP/HTTPS transport timeouts, two retries and strict update
errors. The [offline package controls](../tests/test_build_packages.py) exercise
bounded stall and partial-index behavior during Python test discovery. These
bounds make package acquisition fail within a known limit; they do not guarantee
network recovery. Source-bound PR job outcomes remain in the
[verification ledger](verification/coderabbit-follow-up.md) and PR description.

The [Dockerfile](../Dockerfile) runs locked Rust tests, Clippy with warnings denied and the release build. [Checks CI](../.github/workflows/checks.yml) runs client syntax/tests, real browser DOM regressions, locked Python lint, Compose validation, both image builds, context/integration and controlled DNS-failure checks, Python tests and the live dependency audit. Linux CI also exports both native binaries and actual compiler metadata from the Docker build and runs the native ownership/runtime suite with `FRAYD_NATIVE_REQUIRE_QUALIFICATION=1`; missing inputs fail instead of silently skipping. Default Python discovery in the dependency-audit job still checks packaging/static policy and explicitly skips helper or actual-server scenarios when their inputs are absent. See [native reproducible checks](verification/native-bundle.md#reproducible-checks) for the required inputs. Native Cargo commands, `git diff --check` and the standalone nginx configuration probe are additional local checks; the workflow does not invoke them separately. [CodeQL](../.github/workflows/codeql.yml) provides separate source/workflow analysis. Remote reviews, coverage and check conclusions must be read for the actual PR head; this guide does not reproduce their hosted results.

The manual [macOS qualification workflow](../.github/workflows/release-mac.yml) uses standard `macos-15` ARM64 and `macos-15-intel` Intel runners, each bounded to 35 minutes. It checks actual architecture and executes each platform's native lifecycle suite. It has no tag trigger, publication or artifact upload, and introduces no new action. The proposed upload action remains rejected pending a qualified dependency check. Actual Mac execution is held until the coordinator accepts the final source and local gates; workflow configuration is not a Mac pass. Keep final source/image and architecture results in the [PR description](https://github.com/ColterD/Frayd/pull/2).

## Disposable QA scope

The integration script defaults to `frayd-qa:local` and `frayd-proxy-qa:local`; `--app-image` and `--proxy-image` override them. It creates a unique disposable project, random loopback port and synthetic media. It checks first-owner setup, scans, comic/EPUB replacement, bounded and suffix Range bytes, unsatisfiable ranges, HEAD responses, saved progress, recreation and logout. It also checks proxy recovery after an app IP change, responsive read paths during a blocked scan, and actual codec conversion into writable storage while sources remain read-only. Source replacement checks distinguish fresh converted bytes from stale cache output while retaining IDs and progress.

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

### Dashboard and artwork qualification

The seven dashboard shelves share `categoryShelf()` in `client/js/app.js`: newest
first by default, one per-category oldest toggle, and folder ordering by the
maximum/minimum actual member `created_at`. Numeric TV episode order and the
render-scoped progress reader remain separate. `artwork()` uses the thumbnail
API and a URL version derived from the encoded `artwork_version` field, never a
cache path. The library/search API exposes that nullable opaque basename only
for the same media ID's canonical UUID or retained legacy name. Missing art falls
back to a category symbol; an artwork image does not
establish a title/artist match.

Scanner tests in `server/src/scanner.rs` exercise seven local-source kinds,
missing-cover repair, guarded publication, replacement and conservative
retirement. `server/src/main.rs` covers thumbnail MIME and deletion. Run
`cargo test --locked --manifest-path server/Cargo.toml` and the strict Clippy
command in [canonical checks](#canonical-checks); run the Node and real-DOM
commands there for dashboard sort, representative folder art and the 320×568
footer reachability oracle. The integration test reads the database-selected
UUID JPEG path and asserts exact bytes, rollback preservation and 204 cleanup;
the older `id.jpg` expectation failed before that test correction. EPUB DOM
controls check fresh-frame publication and rejection of retired messages. These
fixture checks do not replace current-image browser acceptance. The filename-first
online iTunes path is **UNWIRED**:
recover `44d1ed7` `extract_thumb`/`fetch_online_thumb`/`itunes_artwork` only
with verified artist/album matching and explicit provider opt-in. There is no
new artwork provider, flag or schema; no general catalog metadata lookup. The
separate EPUB parser dependency is recorded below.

### EPUB bounds and reader layout

The EPUB container/OPF path uses `quick-xml` 0.42.0 with default features off
(`server/Cargo.lock`); its MIT upstream commit is
[`36a2c52`](https://github.com/tafia/quick-xml/tree/36a2c52a4f6c90878c3f60c3c3b5c62efaef28a9),
checked 2026-10-29 and adopted for namespace-aware, bounded parsing. The locked
crate checksum is `41b1177fdf999d2321d3fb46ff47159d9c1fb9ad66a4879f8c50a0b504615e9b`;
it adds one registry package and reuses `memchr` 2.8.3. The local dependency
audit covered 210 packages. This does not certify future advisory absence.

Metadata entries are capped at 2 MiB decompressed, binary entries at 32 MiB;
the stream reads at most limit+1 bytes rather than trusting ZIP headers. XML
has 100,000-event/128-depth bounds, ignores external DTDs and does not expand
custom entities. URI matching decodes once and preserves literal percent names.
Rejection stops cover fallback before FFmpeg/output publication. Cached legacy
art uses a 12-byte sniff and 64 KiB stream chunks; these are per-entry/read
bounds, not an archive, concurrency or RSS budget.

The browser reader waits for fonts and a positive layout before reporting a
page or saving resume; resize can wake an unmeasurable document. Rejected chapter
responses publish a visible unavailable state without a false save. Closing also requires the current ready document, generation and frame identity; pending or failed chapters cannot replace a saved bookmark. Run the
canonical Rust/Clippy/audit commands and `node --test tests/epub-message.test.cjs`
plus the real-DOM regression. Those checks do not prove actual first-open
behavior on a new image or explain Chromium's earlier empty-frame trigger.

### Unwired UI design fragments

`client/index.html` retains five named `<template id="unwired-…">` fragments
beside their intended locations. Template children do not enter the active DOM,
tab order or accessibility tree. The hero fragment sits outside `#hero` because
`renderHero()` replaces that element's contents. No runtime code instantiates
these fragments. Even a manually cloned prototype has disabled controls, explicit
`type="button"` buttons, no control IDs and no event handlers. The working default
continues to expose only implemented actions; these fragments are design intent,
not supported features or a promise of activation.

| Fragment | Removal provenance | Contract required before activation |
| --- | --- | --- |
| `unwired-remember` | `8d879054a16637619b7efc1e1e79ce6d38e568d2` | Define actual token/identity storage, session expiry, unchecked behavior, logout and shared-device policy; the checkbox must change that policy. |
| `unwired-social-auth` (Google / SSO) | `8d879054a16637619b7efc1e1e79ce6d38e568d2` | Implement configured provider routes, redirect/state validation, errors and safe account linking with the local household/account model. |
| `unwired-notifications` | `8d879054a16637619b7efc1e1e79ce6d38e568d2` | Provide an authorized event source, actual notification surface, empty/error states and read/dismiss persistence. |
| `unwired-hero-info` | `829af7d02172fcf36610aacd044699db82d43176` | Open actual selected-media details with close/focus behavior and truthful metadata; preserve the working hero play action. |
| `unwired-audio-navigation` (Previous / Next) | `d49b227b721b0ef20bb87c2e83f148a7e0cb108a` | Define queue order and boundaries, previous-track versus restart behavior, per-track resume/progress ownership and host/guest party authority. |

These are local source-history references checked on 2026-10-29; retrieve the
pre-removal markup with `git show <commit>^:client/index.html` (the hero button
was generated in `client/js/app.js`). SVGs and labels are retained where useful;
removed styling remains recoverable from history. No dependency, configuration,
environment flag or activation path is added. Before activating a fragment,
implement its contract, accessible interaction and affected-path tests, then
move the control into the live UI and qualify the real browser workflow.

`python3 -m unittest discover -s tests -p 'test_unwired_ui.py'` checks the source
boundary and disabled prototype contract. Canonical Python discovery runs this
check in CI; the real DOM suite separately checks default auth/audio controls.

### Party protocol

`POST /api/party` returns a public code and private `host_capability`. Host socket
registration requires that proof plus the creator identity in the current
session generation. The client offers `frayd-party` and `frayd-host.<proof>` as
WebSocket protocols; the server selects only `frayd-party`. Do not put the proof
in URLs, storage, public messages or logs. Ordinary guest joins carry no proof.

Party messages show public `Host` and `Guest N` display labels and carry a
separate `self_label` for the current client's UI. Canonical membership and
authorization identity remains separate from those labels. Refresh open tabs
after upgrading both client and server; mixed-version compatibility is not
qualified by the focused label tests.

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

### Legacy comic page-name compatibility

Persisted ZIP 0.6.6 page labels can differ from ZIP 9 even when the archive bytes are unchanged. The contextual comic reader compares the entire cached list with unique current and historical flag-aware names, preserves its order, and reads the selected physical entry. It never rewrites the cache or user progress. Ambiguous interpretations remain refused; legacy recovery also holds when ZIP reports duplicate central-directory names. Normal canonical subsets retain exact lookup, while directory, RAR and EPUB readers keep their existing paths.

The CP437 high-half is standard encoding data generated with Python's `cp437` codec; no dependency or second ZIP parser was added. Historical algorithm provenance: [zip 0.6.6, MIT, commit 21a20584](https://github.com/zip-rs/zip/blob/21a20584bc9e05dfa4f3c5b0bc420a1389fae2c3/src/read.rs#L1065), checked 2026-10-29; disposition: ADOPT decoding policy, not a second library. Real persisted-cache TCP and archive controls cover old names/order/progress, canonical subsets, flag handling, collisions and conservative refusal.
