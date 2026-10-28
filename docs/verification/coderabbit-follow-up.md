# CodeRabbit follow-up verification

Updated 2026-10-28 UTC for [fork draft PR #2](https://github.com/ColterD/Frayd/pull/2).
This ledger accounts for **18 CodeRabbit issue/diagnostic entries**: the original
12, two initial PR #2 documentation comments, two subsequent cleanup findings,
one outside-diff cache finding and one unverified wording diagnostic. Summary-only
risks and GitHub CodeQL alerts are recorded separately, not added to that count.
PR #2 remains draft and unmerged. No deployment is qualified or authorized here.

## Source and delivery boundaries

| Source | Identity and meaning |
|---|---|
| Original review | [5677254865](https://github.com/ColterD/Frayd/pull/1#pullrequestreview-5677254865), head `f2a46190c83a9d256281204a1db99edd3623983d`. Baseline `08242a83e26a0807af5db8bd89987519b73d9494` has matching server/client source and includes upstream merge `3b0d003bfb604fe49bc627d47aaf7bcefc67c11a`. |
| Initial PR #2 review | [5677483709](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5677483709), documentation-only head `64b546ee712a88292c625028a5867c56755177c0`. |
| First implementation review | [5677717042](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5677717042), head `247ff29fa3771b19e7996b005616475523a2400b`. Two new Minor findings; hosted documentation coverage 100%, 39 functions across 12 files, nine unsupported skips. |
| Latest completed review | [5677830358](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5677830358), head `3b7a5f590bc914ff39f68894be6bdff0d3561617`, submitted 2026-10-28T04:34:52Z, publication batch 1/1 complete. One outside-diff Minor and one unverified Minor diagnostic. |
| Upstream synchronization | Upstream `kattiest/Frayd` commit `540b87acea05304c767a151d47456fceaaf7b29a` merged into fork main at `49de065f3fb0875425f5467b87ae161b43d7914e`, and into the PR branch at `fafad69434a5893cb1d1dbbcbdf56d87f0aab05b`. |
| Security patch already on fork main | PR-branch patch `b840753a9cabb680b94733ae7a58b6b303619d17` was cherry-picked to fork main as `2789f37e55148e0ee69e7bba6bb5ee30b5f49225`. This fixes the original four EPUB-message sender-boundary findings in source; all four default-branch alerts now report fixed, not dismissed. It does not merge PR #2. |
| Current combined work | Implementation `6af4c15` and ancestry-only merge `a6af5f5` include upstream synchronization, hygiene, cache/publication corrections and explicit ingestion trust. Fork main received the narrow ingestion patch at `a7a6dabccce971992d1ed47b00ef66f74811e6e7`. The final app image below passes all 28 Rust tests and full runtime qualification; browser evidence is separately tied to the unchanged client hash. |

Historical runtime images are immutable evidence boundaries:
`sha256:cf030b02bc7703e5e0a5553971d28dac0238c6fc73b22cbd68d6cd99e88e8861`
was used for initial legacy/browser checks;
`sha256:0f7b830c6e19a52cebb5816f9e14f9ae472d9092743157b8ababd35f09c8d03b`
was the later cleanup/namespace build. The proxy was
`sha256:543d71e7db7b987f1e41dfb0a297efe5f020c141b2dbc39a859fd2473df7bbe2`.
These predate the upstream merge and latest backend corrections. Their successful
runs do not qualify the current combined application.

The current backend 20-file manifest has SHA-256
`84f4cec838e4740875a020b5ce1908b86e45762925758143a783595d893a014a`.
Reconciliation rehashed all 20 source inputs with zero mismatches. The final
final Rust run passed 22 unit plus six archive-reader tests (exit 0), Clippy
(exit 0, existing warnings) and release build (exit 0). Both client suites passed
22 tests after the merge. Final normal-image build and runtime qualification also pass;
31 build, nine verification and 20 backend inputs match with zero drift.

## All 18 CodeRabbit dispositions

Severity is the original published severity. “Historical pass” describes the
bounded source/image scenario recorded below; the new combined runtime still needs
qualification. A resolved review thread and a passing check are separate evidence.

| ID | Severity | Verified observation / disposition | Current limit |
|---|---|---|---|
| [CR-01](https://github.com/ColterD/Frayd/pull/1#discussion_r4396472345) | Major | Baseline omitted a song after twenty comics (two new VM cases failed). Filtering playable kinds before the cap passed all 11 original client tests. Real Chrome guest join showed only `Later Music` and its Request button; twenty comics were excluded. | Historical real-browser and current 22-test scripted regressions pass; the picker function is unchanged. Combined picker browser interaction, request relay and decoder behavior were not separately retested. |
| [CR-02](https://github.com/ColterD/Frayd/pull/1#discussion_r4396472349) | Major | Baseline scan test exited 101 while the app DB mutex was held. Separate WAL worker connection passes. Disposable runtime explicitly checked health, static root, library and progress through the proxy before, during and after blocked ffprobe; all twelve returned expected data in 0–5 ms in the later run, and release completed scanning. | Historical unit/runtime pass. Timing is fixture-specific; no proxy-startup requirement is inferred. |
| [CR-03](https://github.com/ColterD/Frayd/pull/1#discussion_r4396472357) | Major | Baseline proxy returned HTTP 502 after app IP replacement and failed its 20s deadline. Later corrected run changed app IP `192.168.112.2` to `.4`; unchanged proxy ID/start time recovered in 8.51s with query/range bytes and WebSocket handshake preserved. | Historical pass; full watch-party protocol not qualified. |
| [CR-04](https://github.com/ColterD/Frayd/pull/1#discussion_r4396472368) | Major | Actual ffmpeg converted synthetic H264/AC3 to H264/AAC under private `/data/transcodes/<media-id>/<job-uuid>/audio.mp4` with a read-only library. Source hash/names stayed unchanged; converted stream bytes survived app restart. Cache/registry failure and overlap controls fail closed. | Final immutable-image conversion and restart stream proof pass; negative ownership/cache controls also pass. |
| [CR-05](https://github.com/ColterD/Frayd/pull/1#discussion_r4396472379) | Minor | Three pagination regressions failed before correction. Any nonempty OSV continuation token now fails before classification; controlled incomplete CLI response exits 1 with zero report bytes and an explicit diagnostic. | Local controlled HTTP-boundary pass; no live paginated response claimed. |
| [CR-06](https://github.com/ColterD/Frayd/pull/1#discussion_r4396472389) | Major | Alleged ZIP 9 UTF-8 decoding failure was not reproduced. Unflagged UTF-8 name lookup and saved physical-page index characterization pass. A separate persisted/fresh page-order mismatch failed (exit 101), then passed after listing/content shared persisted order. | Encoding allegation remains un-reproduced. Changed-archive invalidation is a separate valid issue, NEXT-01, corrected with local handler tests and real combined-image HTTP replacement proof. |
| [CR-07](https://github.com/ColterD/Frayd/pull/1#discussion_r4396472409) | Major | Baseline removed one genuine browser-suffixed row (three instead of four); progress-bearing rows survived FK enforcement. Corrected tests retain four IDs, three progress positions and bytes. A legacy-pointer collision was also reproduced and fixed. | Only exact `provenance='generated'` entries grant row-deletion authority. Legacy playback references, suffixes and ambiguous historical names never do. |
| [CR-08](https://github.com/ColterD/Frayd/pull/1#discussion_r4396472409) | Minor | Exact registered generated rows can be pruned even when the file is absent. Progress, annotations, tracks, playlists, competing ownership and legacy-only provenance retain rows. | Historical fixture pass; no broad orphan deletion or cascading user-data deletion. |
| [CR-09](https://github.com/ColterD/Frayd/pull/1#pullrequestreview-5677254865) | Trivial | Baseline diagnostics failed. Tests now expose byte/text stderr and preserve the original error through cleanup errors; a real synthetic executable exits 23 with its diagnostic intact. Cleanup failure after success stays fatal. | Subprocess pass; no actual Docker outage induced. |
| [CR-10](https://github.com/ColterD/Frayd/pull/1#pullrequestreview-5677254865) | Trivial | Original ignore rules failed. Real context export excludes synthetic `.env` variants and installer sentinels while retaining normal JS; runtime checks image/HTTP exclusion. | Historical Docker pass; repeat context/runtime checks on new build. |
| D-01 | Warning | Original B905 reproduced. Explicit `zip(..., strict=True)` passes selected Ruff rules; mismatched package/result counts remain fatal. | Local and historical hosted lint pass. |
| D-02 | Warning | Latest completed hosted review reports **100.00% >=80.00%**, 44 functions across 12 files, one unsupported skip, on `3b7a5f5`. Threshold unchanged. | Passed for that SHA; next combined-head coverage pending. Exact 12 analyzed filenames are undisclosed; selected files are not the coverage-file list. |
| [PR2-DOC-01](https://github.com/ColterD/Frayd/pull/2#discussion_r4396630072) | Minor | Explicit oracle now requires successful proxy-facing liveness/library/progress before, during and after the blocked probe, followed by scan completion. Historical harness rerun exercised all phases. | Documentation and historical verification gap closed; separate from CR-02's Major code severity. |
| [PR2-DOC-02](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5677483709) | Trivial | Explicit oracle preserves cached page names, page-index archive lookup, IDs, metadata and progress after upgrade/rescan. Populated legacy startup refreshed two rows; explicit rescan returned HTTP 202 then idle. All 19 HTTP/eight DB assertions passed after each. | Historical pass for unchanged archives; NEXT-01 covers archive replacement. |
| [FRESH-01](https://github.com/ColterD/Frayd/pull/2#discussion_r4396816530) | Minor | Old cleanup missed a real stopped labelled container. Corrected all-state listing rejects it; removal restores clean checks. Red regression followed by 14 passing Python tests and lint. | Hosted thread resolved/outdated, marked addressed in `3b7a5f5`; historical runtime cleanup passed. |
| [FRESH-02](https://github.com/ColterD/Frayd/pull/2#discussion_r4396816549) | Minor | Two failed jobs originally left four registry rows. Corrected tests require zero per-job row/directory growth across repeat failure while retaining neighbors/legacy artifacts; historical 24 Rust tests and runtime passed. | Hosted thread resolved/outdated, marked addressed in `3b7a5f5`. Later publication-failure risk is separate below. |
| [NEXT-01](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5677830358) | Minor, outside diff | Valid: persisted comic pages can outlive source mtime/size changes. Required oracle replaces an archive, rescans, verifies added page listing/index bytes while retaining ID/progress/other metadata. | Corrected in the final 28-test suite and real combined-image HTTP replacement test. Seconds-granularity mtime plus size does not detect replacements preserving both values. |
| [NEXT-D01](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5677830358) | Minor, unverified tool diagnostic | Report requested “Ubuntu-hosted” wording but explicitly could not confirm its finding. Ledger uses **“hosted run on Ubuntu”** and records the actual historical workflow outcome below. | Wording clarified; diagnostic validity is not claimed. |

The latest review selected five files, and explicitly skipped
`tests/test_integration.py` as similar to previous changes. Selection does not prove
full analysis. No path-filter ignored section was published. The original review
contained eight actionable items across seven inline records, two nested nitpicks
and two diagnostics; CR-07/08 share a comment but are separate requirements.

## Summary-only risks and separate security work

These are outside the 18-entry CodeRabbit count:

- **Prolonged DB lock:** cleanup must retain provenance if ownership cannot be
  validated. Historical follow-up tests held a lock beyond the normal busy timeout
  and retried after release. Persistent lock can leave a reported held job; there
  is no automatic global orphan reaper.
- **Failed publication:** latest review summary identified a real unused-output
  leak, separate from FRESH-02's conversion failure. A SQLite trigger refused
  publication after process doubles produced a completed file; two jobs left four
  records and unused outputs before correction (exit 101). Final test retires only
  exact unreferenced generated current-job files; repeated failure leaves zero
  records/job directories, unchanged source bytes and NULL playback pointer.
  Legacy/referenced/neighbor files remain protected. All 28 Rust tests and the
  final-image runtime suite pass. Lock/permission/ownership conflicts remain held.
- **Cache namespace containment:** historical negative test exposed a media-ID
  symlink escaping the cache. Correction rejects symlink/non-directory parents and
  atomically creates a UUID directory before registration. Private cache ownership
  remains required; a hostile same-UID parent-replacement race is not qualified.
- **Six PR CodeQL path/command diagnostics:** these were separate from CodeRabbit.
  Source triage traced the route to server-created application state, fixed
  executables and numeric ID/UUID/fixed-name output components. A zero-argument
  callback made the boundary explicit; no suppression/dismissal was added. The
  [CodeQL aggregate on `3b7a5f5`](https://github.com/ColterD/Frayd/runs/122632053805)
  completed successfully with “No new alerts in code changed by this pull request”
  and zero annotations. This clears the PR-check gate; individual global alert
  states for those six were not established by this aggregate. The result predates
  the new combined source.
- **Four actual default-branch EPUB-message findings:** reproduced hostile sender
  cases triggered annotation/progress/chapter requests before the fix. The guard
  now requires an active reader, exact `null` origin, exact active-frame
  `event.source`, and valid payload shape before dispatch. Five hostile classes
  went from failing to producing no effects. Combined client suite passed 22/22,
  including legitimate highlights, progress, search/geometry and chapter changes.
  Fork main received the security patch separately as recorded above. Alerts
  [1](https://github.com/ColterD/Frayd/security/code-scanning/1),
  [2](https://github.com/ColterD/Frayd/security/code-scanning/2),
  [3](https://github.com/ColterD/Frayd/security/code-scanning/3) and
  [4](https://github.com/ColterD/Frayd/security/code-scanning/4) each report
  `fixed` at 2026-10-28T04:40:11Z on main, not dismissed. Main's JavaScript CodeQL
  job passed; Rust/build jobs were still running in that snapshot. Alert instance
  commit `49de065` identifies the last unsafe instance, not the fixing scan head.
  PR head `d9c91eb` includes the main fix, so it is already in the PR base rather
  than new PR-only security work. Real-browser sandbox and legitimate navigation/
  resume qualification passed on the unchanged current client. Trust of scripts already inside the
  active EPUB frame and same-frame navigation generations are unchanged limits.

## New archive-replacement qualification

The valid NEXT-01 baseline was reproduced after the upstream merge: unchanged
rescan retained persisted reverse order `[page10,page2]`, then replacement with an
added `page5` and deliberately restored mtime still returned the stale two-page
cache (exit 101). The corrected scanner compares both size and mtime with
NULL-safe semantics, clearing only changed comic/manga page names. Backdated and
missing legacy fingerprints are covered; unchanged sources retain persisted order.
Lazy publication also compares the original DB fingerprint against any intervening
rescan before writing names.

In the final handler test, rescan after replacement returns
`[page2,page5,page10]`; page index 1 returns exact `new fifth image` bytes. ID 1,
position 7, title and synopsis remain unchanged. Malformed metadata is retained;
unchanged scans report zero updates. These tests invoke actual Axum handlers with
response bodies, not a network listener. The combined-image network HTTP replacement test also
passed as recorded below. The final-image removed-page check also passes,
including exact runtime timestamp and database preservation. A source replacement preserving both second-level mtime
and exact byte length is outside this fingerprint's detection boundary.

## Historical qualification and current hygiene

The later pre-upstream image build passed **18 unit + 6 archive-reader tests**,
Clippy and release build. Its integration exited 0: first-run setup, duplicate
rejection, authenticated session/login/logout, full/range bytes, saved progress,
recreation, forced IP change, WebSocket upgrade, twelve blocked-scan phase reads,
real read-only-source conversion, restart persistence and all-state cleanup.
Clippy retained existing warnings; this is not a warning-free claim.

The original hosted integration failed because Docker on Ubuntu rejected a fixed
reservation IP on a daemon-allocated subnet. The correction uses dynamic allocation
while preserving changed-IP and unchanged-proxy assertions. The later **hosted run
on Ubuntu** and all hosted checks on `3b7a5f5` passed: all ten check runs completed successfully, including
[build/integration](https://github.com/ColterD/Frayd/actions/runs/40632502363/job/122631898658),
[dependency audit](https://github.com/ColterD/Frayd/actions/runs/40632502363/job/122631898423),
four language/workflow analyses, aggregate CodeQL and CodeRabbit (push/PR checks
include duplicate workflow jobs). Earlier failure logs remain historical controls;
they are not current gates.

Legacy fixture proof used baseline schema plus annotations and no initial artifact
registry. Nineteen curl requests each exited 0 with expected statuses/bodies; eight
DB invariants passed after startup/restart and populated-library rescan. Cached
`["Á.jpg","À.jpg","Â.jpg"]` mapped index 0 to acute bytes and index 1 to grave;
uncached natural order had matching bytes. All seven media IDs, two progress rows,
annotation 19, synopsis/custom/chapter metadata survived. One backfilled
`legacy_reference` preserved a genuine shared final path; genuine final/temp suffix
rows streamed exact markers. These were byte-transport fixtures, not decoded media.

The historical Chrome guest picker showed `(guest)` and only `Later Music` plus
Request after twenty comics. Request relay/approval and playback were not exercised.
Owned preview containers/networks were removed; former ports are not demo endpoints.

Hygiene at the synchronized source adds root ignore rules for native
`server/transcodes/` and `/.ruff_cache/`, removes unused `users-three.svg` after
no-reference checks on both fork/upstream source, and documents browser YouTube
iframe access separately from server URL ingestion. Stock Docker lacks yt-dlp and
its app network is internal; native ingestion needs networking and writable media.
These are source/config observations, not live ingestion acceptance. Vendor Plyr,
all seven referenced archive fixtures and excluded experimental installers remain.
No new application dependency was added. Hygiene receipt records 22 client tests,
14 Python tests, selected Ruff rules, syntax and whitespace checks passing after
merge; these do not stand in for combined Rust/image/runtime qualification.

## Repeatable commands and remaining gates

Run from an isolated checkout of the exact commit being qualified. Docker commands
create QA images and disposable projects; keep tags separate from deployment.
The pinned Docker builder runs locked Rust tests, Clippy and release build.

```sh
node --check client/js/app.js
node --check client/js/party.js
node --test tests/*.test.cjs
python3 -m unittest discover -s tests -p 'test_*.py' -v
docker compose config --quiet
git diff --check
```

The latest recorded client/Python counts are 22/14 passing respectively, exit 0.
VM tests execute full scripts with synthetic browser/network boundaries; they do
not establish actual browser sandbox behavior. CI now runs both client suites.

For the Linux x86_64 hash-locked lint environment:

```sh
RUNNER_TEMP=$(mktemp -d)
python3 -m venv "$RUNNER_TEMP/frayd-python-lint"
"$RUNNER_TEMP/frayd-python-lint/bin/python" -m pip install --disable-pip-version-check --require-hashes --no-deps --only-binary=:all: -r .github/requirements-ci.txt
"$RUNNER_TEMP/frayd-python-lint/bin/ruff" check --no-cache --select E4,E7,E9,F,B905 scripts tests
```

Historical exit 0. CI-only Ruff 0.17.3 is hash-locked, MIT-licensed; PyPI metadata
and OSV were checked with no indexed advisory returned. Source-to-wheel
reproducibility was not established. Remove only the created temporary directory.

```sh
docker build --progress plain -t frayd-pr2-qa:local .
docker build -f Dockerfile.proxy -t frayd-pr2-proxy-qa:local .
docker run --rm --network none --read-only --tmpfs /tmp --entrypoint nginx frayd-pr2-proxy-qa:local -t
python3 scripts/check-build-context.py
python3 -u scripts/integration.py
python3 scripts/audit-dependencies.py
```

These commands passed historically, but must run again after the new source freeze.
The context check exports an actual image with synthetic `.env` sentinels. The
audit queries current OSV; the historical 209-package clean result and exact
block-buffer 0.10.4 publisher-range exception do not predict future advisories.
CI repeats syntax/client/Python/lint/Compose/build/context/integration/audit checks.

Remaining acceptance gates:

1. Obtain final-head hosted checks, CodeQL and a completed CodeRabbit review,
   including documentation coverage >=80%. The automatic incremental run on
   `9f3b700` was skipped before analysis; its findings/coverage are unavailable.
   The displayed 100% result belongs to `3b7a5f5`, not the skipped or final head.
   A documented full-review command is the next probe. Review conversations stay
   open until their complete acceptance oracles and fresh-review gate pass.
2. Record the remaining CodeQL alert 13 disposition explicitly. Alerts 1-4 and
   11-12 report fixed through source changes; alert 13 still flags the intended
   HTTP(S) URL argument after the explicit option terminator. Its source/parser/
   recorder evidence is retained. The owner approved a false-positive disposition;
   the full rationale and current remote state are recorded in the PR body. This
   does not label it fixed by a source change or authorize query suppression.
3. Retain draft status until owner acceptance. Deployment, production restart,
   real-data migration, arbitrary codecs and full watch-party behavior remain
   outside the qualification recorded here.

## Inherited ingestion diagnostics

After synchronization, GitHub separately reported alerts 11–13 in upstream's
experimental ingestion handler: destination creation, output argument and URL
argument. Their reported source is the route callback. The destination comes
from server configuration; the URL is real request data, restricted to an admin
and HTTP(S), passed to a fixed executable as one argument without a shell. A
bounded refinement now binds trusted application state explicitly and terminates
command options before the URL. An isolated, network-free recorder verifies admin
and scheme rejection, exact arguments, paths with spaces and literal metacharacters.
All 28 Rust tests, Clippy and release compilation pass. The narrow patch is also
on fork main at `a7a6dab`; its separate eight-test Rust suite, Clippy, release and
all hosted workflow jobs pass. GitHub marks alerts 11-12 fixed. Alert 13 remains
open despite the literal option terminator and effective argument controls; its
false-positive disposition has an explicit owner approval, recorded separately
from the source-fixed alerts. The PR body records the approved full rationale
and current GitHub state. These are separate from the four
EPUB findings already fixed on fork main. No query/code suppression was added;
alert 13 receives the separately approved false-positive disposition.

## Real-browser and archive-replacement proof

On immutable combined-source image
`sha256:bd333d3458e8ecd8c6cee4f70021a3367ff773cf39c2b91ed845c74d9bec5d19`,
the production client hash was
`f2e00dedb4aec824e323699b1bceb63d46a08a54b3219d575f7ba845be62599f`.
Real Chrome accepted active-reader messages with opaque origin `null`: chapter
navigation worked, progress saved at 1.25, and reopening resumed chapter two,
page two of five. Six forged highlight/page/chapter reports from unrelated opaque
and same-origin frames caused zero annotation/progress/chapter requests, and
reader UI/database state stayed unchanged. Positive highlight persistence and
media decoding were not tested.

A real HTTP replacement test kept comic 8's exact modification timestamp while
adding a page. After HTTP 202 rescan, the refreshed list and page-1 response
contained the exact added bytes; ID, title, progress, synopsis and custom metadata
remained intact. Both owned containers and their network were removed afterward.
This qualifies the stated image/client. The later ingest-only server refinement
passes separate final build/runtime qualification below; final-head hosted
acceptance remains a separate gate.

## Final immutable-image qualification

The final application is
`sha256:89306784bd39c45379c7cfbad8af9b3ec78adedcca4947eb028b55d3de92090e`;
proxy `sha256:543d71e7db7b987f1e41dfb0a297efe5f020c141b2dbc39a859fd2473df7bbe2`
is unchanged. Normal Docker build passed 28 Rust tests, Clippy and release build.
Node 22/22, Python 14/14, configured CI Ruff, context exclusion and Compose checks
passed. The exact image/client/binary identities and all 31/9/20 input hashes were
rechecked after runtime with no drift.

Full disposable integration exited 0. The app IP changed `.2` to `.4`; the proxy
ID/start time stayed unchanged and HTTP recovered in 9.01 seconds. Query/range
bytes and WebSocket upgrade passed. All twelve before/during/after scan reads
returned expected data in 0–7 ms in this fixture. Actual H264/AC3 to H264/AAC
conversion used a writable cache outside read-only media; source names/hashes and
exact converted HTTP bytes after restart passed. Cleanup included stopped
containers and confirmed zero owned containers, networks or volumes. This is local
qualification, not a deployment or a completed hosted-review claim.

The removed-page scenario used the final `89306784` image with a fresh direct host
bind, avoiding earlier fixture-copy/snapshot failures. All 44 named checks passed:
archive size changed 310 to 201 bytes while host and container both retained
`mtime_ns=1793160000123456789`; `[0.jpg,1.jpg,2.jpg]` refreshed to `[0.jpg,2.jpg]`,
index 1 returned exact `TWO` bytes and index 2 returned HTTP 404. Consistent SQLite
backups preserved media ID 8, all unrelated media columns, identity 17, position 1
and progress timestamp 12345. Four backups passed integrity checks. The earlier
unsuccessful harness attempts remain failed evidence; this fresh scoped approach
passed and both attempts' disposable resources were removed.

Remote acceptance metadata is recorded in the PR body, including full-review
publication, check results and the owner-authorized alert 13 disposition. This
source ledger preserves immutable test boundaries and never treats a skipped
review as acceptance.
