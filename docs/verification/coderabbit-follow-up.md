# CodeRabbit follow-up verification

This ledger covers all eight actionable findings, two nitpicks and two diagnostics
from [review 5677254865](https://github.com/ColterD/Frayd/pull/1#pullrequestreview-5677254865)
on `f2a46190c83a9d256281204a1db99edd3623983d`. The follow-up is
[fork draft PR #2](https://github.com/ColterD/Frayd/pull/2); deployment remains held.
Results below were collected on 2026-10-27 America/Chicago (2026-10-28 UTC).

## Source and verification boundary

The baseline is synchronized fork main
`08242a83e26a0807af5db8bd89987519b73d9494`, including upstream merge
`3b0d003bfb604fe49bc627d47aaf7bcefc67c11a`. Its server/client source matches the
reviewed head. Baseline failures below used that source with disposable fixtures;
there was no access to an operator database or library.

The tested application image is
`sha256:cf030b02bc7703e5e0a5553971d28dac0238c6fc73b22cbd68d6cd99e88e8861`;
the proxy is
`sha256:543d71e7db7b987f1e41dfb0a297efe5f020c141b2dbc39a859fd2473df7bbe2`.
These identities bind the reported local runtime checks, rather than an untested
moving branch head. All 20 backend and 32 image-build input hashes were rechecked
against the final local source with zero mismatches. Publication and hosted checks
must bind their results to the eventual PR commit. Private local receipts are
summarized here with the actual observations; the committed tests and commands
below allow repetition without those receipts.

## Finding dispositions

“Local pass” means the bounded scenario passed, not hosted acceptance or deployment.

| ID | Original review severity | Baseline and final observation | Disposition / limit |
|---|---|---|---|
| CR-01 | Major | With twenty comics before a song, the baseline picker omitted the song; two new VM scenarios failed (9/11 passing). Filtering playable kinds before the cap passes all 11 tests, preserving order, escaping titles and limiting to 20 playable choices. | Local scripted and real Chrome guest-picker pass; no decoder or request-relay claim. |
| CR-02 | Major | Baseline scan test exited 101 while the app DB mutex was held. A separate WAL worker connection passes. With a deliberately blocked real `ffprobe`, successful proxy-facing health, root, library and progress HTTP reads completed in 0.000–0.001s in the recorded run; releasing it completed the scan. | Local unit and disposable HTTP pass; timings describe this fixture only. Full before/during/after oracle is specified below with its remaining limit. |
| CR-03 | Major | Baseline proxy returned HTTP 502 after a forced app IP change and failed the 20s readiness deadline (exit 1). Final proxy recovered in 9.01s after the IP changed; its container ID and start time were unchanged. Query/range bytes and WebSocket upgrade survived. | Local Docker pass; no full watch-party protocol qualification. |
| CR-04 | Major | Real packaged ffmpeg converted synthetic H264/AC3 to H264/AAC under `/data/transcodes/<media-id>/<job-uuid>/audio.mp4` with an OS-enforced read-only media mount. Original hash/names were unchanged; exact converted HTTP bytes survived app restart. Cache/registry failure and library overlap tests fail closed. | Local runtime and negative-test pass; no all-format or browser decoder claim. |
| CR-05 | Minor | Three pagination regressions failed before the correction. Now any nonempty continuation token is rejected before classifying results, with or without first-page findings. The actual CLI entry point under a controlled incomplete response exits 1 with zero report bytes and an explicit incomplete-audit diagnostic. | Local controlled HTTP-boundary pass; no live paginated OSV response claimed. |
| CR-06 | Major | The alleged ZIP 9 UTF-8 decoding regression was not reproduced: unflagged UTF-8 name lookup and saved physical-page index characterization pass. A separate cached/fresh ordering mismatch was reproduced (exit 101) and corrected: listing and content now honor the same persisted order. Missing-cache population preserves other metadata; unreadable archives do not erase it. | Allegation not reproduced; separate coherency defect fixed and locally verified. See compatibility proof below. |
| CR-07 | Major | Baseline scanner removed one genuine browser-suffixed row (3 rather than 4); its progress-bearing rows survived FK enforcement. Final tests retain all four IDs, three progress positions and source bytes. A separate legacy-reference collision initially removed a genuine row; the corrected guard retains its ID/bytes and existing playback pointer. | Local pass. Only `provenance='generated'` permits automatic artifact-row deletion; a legacy playback reference never grants deletion authority. |
| CR-08 | Minor | Tests prune exact registered generated artifact rows whether their files exist or are missing. Rows with progress, annotations, tracks, playlists, competing ownership or legacy-only provenance remain retained. | Local fixture pass; ambiguous unregistered historical names are deliberately preserved. |
| CR-09 | Trivial | Baseline diagnostic tests failed. Final tests expose captured byte/text stderr, preserve the original error through cleanup failures, and exercise a real synthetic Docker executable exiting 23 with its diagnostic intact. Cleanup failure after success remains fatal. | Local subprocess pass; no actual Docker outage induced. |
| CR-10 | Trivial | Original ignore rules failed the sentinel context oracle. Final real scratch-image/export check excludes synthetic `.env` variants and installer sentinels while retaining normal JS. The disposable runtime also checks image/HTTP exclusion. | Local Docker pass; sentinels contain no real secrets. |
| D-01 | Lint | Original B905 reproduced. Explicit `zip(..., strict=True)` passes selected Ruff rules; result-count mismatch remains fatal. | Local pass, including the exact CI lint block. |
| D-02 | Documentation | Meaningful docs added to changed production callables and new helpers. Local AST count is 3/3 audit script and 13/13 named audit-test callables/classes documented. | **Pending hosted CodeRabbit result >=80%.** Local AST counting is not the hosted metric; the threshold was not lowered. |

## Initial PR #2 documentation review

[Review 5677483709](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5677483709)
covered only the initial ledger commit
`64b546ee712a88292c625028a5867c56755177c0`, not the implementation now described.
Its two documentation findings are distinct from the original code severities:

- **Minor, [4396630072](https://github.com/ColterD/Frayd/pull/2#discussion_r4396630072):**
  CR-02's explicit acceptance oracle is successful **proxy-facing liveness, library
  and progress reads before, during and after** a deliberately blocked metadata
  probe, followed by scan completion. No proxy startup requirement is inferred.
  The updated runtime harness explicitly checks all three surfaces plus the static
  root through the proxy before, during and after the blocked scan. All twelve
  reads returned expected data in 0–8 ms in the disposable rerun, including saved
  progress after release; scanning completed. This closes the documentation and
  phase-verification gap, separate from CR-02's historical Major label.
- **Nitpick, `53807223294e2a90f1e418b8`:** CR-06's acceptance oracle explicitly
  preserves cached page names and archive lookup by page index **after upgrade and
  rescan**, together with media IDs, metadata and progress. The compatibility proof
  below records final-image startup/restart results and persisted/fresh ordering;
  committed tests cover scanner identity preservation and archive lookup. A
  separate populated legacy-library check ran startup scan (two existing rows
  refreshed) and an explicit HTTP 202 rescan. All 19 HTTP and eight database
  assertions passed after each; the unchanged explicit rescan required zero
  upserts. This closes the wording and verification gap without converting the
  un-reproduced ZIP allegation into a proven bug.

## Repeatable checks

Run from the repository root on an isolated checkout. Docker commands create QA
images and disposable projects, and require a functioning local Docker daemon.
The Rust build uses the pinned builder in `Dockerfile`; no host Rust installation
is required. Keep the QA tags separate from any deployed images.

```sh
node --check client/js/app.js
node --check client/js/party.js
node --test tests/client.test.cjs
python3 -m unittest discover -s tests -p 'test_*.py' -v
docker compose config --quiet
git diff --check
```

Recorded result: every command exited 0; 11 client tests and 13 Python tests passed.
The client and Python suites were rerun while reconciling this ledger, with the
same counts and exit 0. Client DOM/network boundaries are synthetic VM fixtures.
Python tests cover audit pagination/count/publisher policy and subprocess errors.

Repeat the exact Linux x86_64 CI lint block in a disposable environment:

```sh
RUNNER_TEMP=$(mktemp -d)
python3 -m venv "$RUNNER_TEMP/frayd-python-lint"
"$RUNNER_TEMP/frayd-python-lint/bin/python" -m pip install --disable-pip-version-check --require-hashes --no-deps --only-binary=:all: -r .github/requirements-ci.txt
"$RUNNER_TEMP/frayd-python-lint/bin/ruff" check --no-cache --select E4,E7,E9,F,B905 scripts tests
```

Recorded exit 0, including the hash-locked Ruff 0.17.3 install. This CI-only MIT
wheel was checked against PyPI metadata and OSV (no indexed advisory returned);
source-to-wheel reproducibility and hosted runner execution were not established.
Remove only the temporary directory created by the command after inspection.

```sh
docker build --progress plain -t frayd-pr2-qa:local .
docker build -f Dockerfile.proxy -t frayd-pr2-proxy-qa:local .
docker run --rm --network none --read-only --tmpfs /tmp --entrypoint nginx frayd-pr2-proxy-qa:local -t
python3 scripts/check-build-context.py
python3 -u scripts/integration.py
python3 scripts/audit-dependencies.py
```

Recorded exit 0 for each command. The application build runs `cargo test --locked`
(14 unit + 6 archive-reader tests), `cargo clippy --locked --all-targets` and
`cargo build --locked --release`. Clippy retained seven existing binary warnings
and five duplicate integration warnings; there is no claim of warning-free Rust.
The live OSV run checked 209 locked packages with no applicable finding and retained
the existing exact publisher-range exception for block-buffer 0.10.4. That result
is time-bound; the command queries current OSV and may fail on new advisories.

The integration command exercises first-run setup, duplicate rejection, login,
logout/session revocation, comic bytes, full/range streaming, progress persistence,
app recreation, forced DNS/IP replacement, WebSocket handshake, blocked scan and
real conversion against read-only media. It removes its owned containers, networks
and volume in cleanup; the final recorded run confirmed their absence. The context
check exports an actual temporary image rather than simulating ignore matching.
Existing `.github/workflows/checks.yml` repeats these client/Python, lint, build,
context, runtime and audit checks on pushes and PRs. Local results do not establish
that the hosted workflow has run on the final PR head.

For the focused backend oracles, run the following inside the same Rust builder
with the `server/` tree as working directory, or with a matching local Rust setup:

```sh
cargo test --locked scan_uses_worker_connection_while_application_database_is_locked
cargo test --locked listing_honors_persisted_page_order_used_by_content
cargo test --locked legacy_playback_collision_preserves_genuine_media_identity_and_bytes
cargo test --locked scan_preserves_genuine_browser_suffix_sources_and_progress
cargo test --locked registered_present_and_missing_rows_prune_but_all_user_references_and_conflicts_survive
cargo test --locked --test archive_readers
```

All named scenarios passed in the recorded final suite. The first four supplied
useful failing controls against baseline or the intermediate collision guard;
the ZIP characterization cases passed and do not establish an encoding regression.

## Legacy compatibility HTTP proof

A second disposable app started from the baseline SQLite schema plus annotations,
with no artifact registry, persisted UTF-8 comic page order, progress, metadata,
and genuine browser-suffixed media. It used the final application image above.
After startup and again after restart, 19 exact HTTP requests (each curl exit 0)
and eight DB invariants passed:

- Cached listing returned `["Á.jpg","À.jpg","Â.jpg"]`; page 0 returned the acute
  marker and page 1 the grave marker. The uncached archive populated natural order
  `["À.jpg","Á.jpg","Â.jpg"]` with matching page bytes.
- Negative/out-of-range pages, absent archive and absent media returned expected
  HTTP 404s. Existing progress remained position 1, updated_at 12345.
- All seven original media IDs, both progress records, annotation 19, synopsis,
  custom metadata and chapters remained intact. Missing-cache population changed
  only pages; an absent archive left existing metadata intact.
- A legacy final pointer and the genuine row sharing that path both streamed
  `LEGACY-FINAL-PLAYBACK` with HTTP 200. Genuine final/temp suffix rows also streamed
  exact marker bytes. The single backfilled registry entry was `legacy_reference`;
  repeated startup preserved it and every media ID.

These are synthetic marker-byte transport and SQLite migration observations,
not decodable image/video playback. The committed archive, scanner, cache and
collision tests above repeat the core invariants without private QA harness files.
A populated-library follow-up refreshed two legacy comic rows during startup,
then accepted an explicit rescan (HTTP 202, followed by scanning=false). All 19
HTTP and eight DB assertions passed after both scans; persisted page order and
bytes, IDs, progress, annotations and custom metadata survived. The explicit
unchanged rescan required zero upserts. These synthetic fixtures do not replace
testing an owner's actual legacy database. All task-owned
legacy QA containers/networks were removed; no live deployment was modified.

## Browser picker proof

**Browser picker passed:** real Chrome on the manually opened disposable page:
   Continue as guest, Party, join a distinct synthetic host, observe `(guest)` and
   `REQUEST A TRACK` containing only `Later Music` with its Request button. Twenty
   comics were excluded and song 99 was selectable. The rendered screenshot and
   action receipt were reconciled; all labelled preview containers/networks were
   removed. Track request relay, host approval and browser media decoding remain
   untested. This fixture has been removed; its former port is not a demo endpoint.

## Outstanding acceptance gates

1. **Hosted CodeRabbit D-02:** run the next review on the final PR commit and record
   its actual function documentation coverage, requiring at least 80%. Local docs
   and AST counts are supporting evidence only. A successful review status alone
   does not close this gate or any unobserved scenario.
2. **Hosted final-head checks and owner acceptance:** confirm both workflow jobs on
   the published PR commit and retain draft status until acceptance. No deployment,
   production restart, real-data migration, browser decoder qualification, arbitrary
   codec support or complete watch-party protocol result is claimed here.
