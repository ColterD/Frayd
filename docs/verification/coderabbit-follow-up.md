# CodeRabbit follow-up verification

[Documentation index](../README.md) · [Operations](../operations.md) · [Development and canonical checks](../development.md)

This is the historical verification ledger, not a setup guide or a current-head
acceptance report. Use [Development](../development.md#canonical-checks) for the
maintained command list. Every result below applies only to its recorded source,
image and scenario. Earlier pending gates describe their historical checkpoint;
they do not reopen work completed later. Hosted acceptance for the current head belongs to
the [draft PR description](https://github.com/ColterD/Frayd/pull/2). Draft status
and no-deployment boundaries remain in force until owner acceptance.

Navigation: [source boundaries](#source-and-delivery-boundaries),
[dispositions](#coderabbit-dispositions),
[historical commands and gates](#historical-commands-and-acceptance-gates),
[EPUB follow-up](#epub-replacement-coherence-follow-up),
[comic cache follow-up](#optional-comic-page-cache-write-follow-up).

Updated 2026-10-28 UTC for [fork draft PR #2](https://github.com/ColterD/Frayd/pull/2).
This ledger accounts for **48 CodeRabbit issue/diagnostic entries**, including
original and subsequent findings, outside-diff comments, nitpicks and published
tool diagnostics. Summary-only risks and GitHub CodeQL alerts are recorded
separately and are not added to that count.
PR #2 remains draft and unmerged. No deployment is qualified or authorized here.

## Source and delivery boundaries

| Source | Identity and meaning |
|---|---|
| Original review | [5677254865](https://github.com/ColterD/Frayd/pull/1#pullrequestreview-5677254865), head `f2a46190c83a9d256281204a1db99edd3623983d`. Baseline `08242a83e26a0807af5db8bd89987519b73d9494` has matching server/client source and includes upstream merge `3b0d003bfb604fe49bc627d47aaf7bcefc67c11a`. |
| Initial PR #2 review | [5677483709](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5677483709), documentation-only head `64b546ee712a88292c625028a5867c56755177c0`. |
| First implementation review | [5677717042](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5677717042), head `247ff29fa3771b19e7996b005616475523a2400b`. Two new Minor findings; hosted documentation coverage 100%, 39 functions across 12 files, nine unsupported skips. |
| Earlier completed review | [5677830358](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5677830358), head `3b7a5f590bc914ff39f68894be6bdff0d3561617`, submitted 2026-10-28T04:34:52Z, publication batch 1/1 complete. One outside-diff Minor and one unverified Minor diagnostic. |
| Earlier whole-PR full review | [5678108505](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5678108505), head `67bf71a79a50419047f9899354698e3e37f363ef`, publication batch 1/1. One Minor index finding; zero outside-diff, nitpick or tool warnings. Hosted documentation coverage 100%, 47 functions across 12 files, ten unsupported skips. |
| Whole-PR review on `5451980` | [5678577205](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5678577205), head `54519804a27f5139b7ee54c10b604e898af355d9`, batch 1/1, 24 selected files and one SVG excluded. Three Minor documentation/harness issues; zero outside-diff items, nitpicks or tool diagnostics. Coverage 100%, 54 functions across 12 files, ten unsupported skips. |
| Whole-PR review on `23ba22e` | [5678783997](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5678783997), batch 1/1, one Minor proxy-command alignment; zero outside-diff items, nitpicks or tool diagnostics. Coverage 100%, 56 functions across 12 files, ten unsupported skips. All four preceding review threads were marked addressed and resolved. |
| Whole-PR review on `9a22ad2` | [5678944739](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5678944739), batch 1/1, one Minor legacy-reference scanner exclusion; zero outside-diff items, nitpicks or tool diagnostics. Coverage 100%, 56 functions across 12 files, ten unsupported skips. The prior proxy-command thread was addressed and resolved. |
| Whole-PR review on `a634a5f` | [5679275186](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5679275186), batch 1/1, one Minor image-provenance documentation request; zero outside-diff items, nitpicks or tool diagnostics. Coverage 100%, 58 functions across 12 files, ten unsupported skips. Legacy-refresh runtime thread addressed and resolved. |
| Whole-PR review on `6028ec0` | [5679402211](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5679402211), batch 1/1, one Minor EPUB spine invalidation issue; zero outside-diff items, nitpicks or tool diagnostics. Coverage 100%, 58 functions across 12 files, ten unsupported skips. Image-provenance thread addressed and resolved; all ten checks passed. |
| Whole-PR review on `041b4cc` | [5679720492](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5679720492), batch 1/1, two Minor historical-evidence/comic-cache-write items; zero outside-diff items, nitpicks or tool diagnostics. Coverage 100%, 64 functions across 12 files, ten unsupported skips; all ten checks passed. |
| Whole-PR review on `d835f40` | [5683790007](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5683790007), batch 1/1, three actionable items: two Minor selection/backup findings and one Major stale-reservation finding. Zero outside-diff items, nitpicks or tool diagnostics. Coverage 90.41%, 73 functions across 14 files, 14 unsupported skips; required threshold remains 80%. The separate CodeQL finding concerns generated-wrapper extraction in a test and is recorded separately from this count. |
| Whole-PR review on `a95409c` | [5684707616](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5684707616), batch 1/1, one Minor backup-checksum documentation finding. Zero outside-diff items, nitpicks or tool diagnostics. Coverage 90.48%, 84 functions across 14 files, 14 unsupported skips; threshold remains 80%. All ten checks passed; the previous four conversations were resolved. |
| Upstream synchronization | Upstream `kattiest/Frayd` commit `540b87acea05304c767a151d47456fceaaf7b29a` merged into fork main at `49de065f3fb0875425f5467b87ae161b43d7914e`, and into the PR branch at `fafad69434a5893cb1d1dbbcbdf56d87f0aab05b`. |
| Security patch already on fork main | PR-branch patch `b840753a9cabb680b94733ae7a58b6b303619d17` was cherry-picked to fork main as `2789f37e55148e0ee69e7bba6bb5ee30b5f49225`. This fixes the original four EPUB-message sender-boundary findings in source; all four default-branch alerts now report fixed, not dismissed. It does not merge PR #2. |
| Combined foundation | Implementation `6af4c15` and ancestry-only merge `a6af5f5` include upstream synchronization, hygiene, cache/publication corrections and explicit ingestion trust. Fork main received the narrow ingestion patch at `a7a6dabccce971992d1ed47b00ef66f74811e6e7`. The pre-index app image below passes 28 Rust tests and full runtime qualification; the latest index follow-up passes 29 Rust tests, Clippy and release build. Post-index image/review results are recorded in the PR body; browser evidence stays tied to the unchanged client hash. |
| Historical source-version follow-up | Follow-up to `b2a8504` corrects a validated summary observation: known source replacements invalidate current playback, and jobs publish only when actual file/database versions stay unchanged. All 32 Rust tests, Clippy and release build pass. Immutable image `a19b1132` and the real-codec red/green regression qualify this source; fresh hosted acceptance is recorded in the PR body. |

Historical runtime images are immutable evidence boundaries:
`sha256:cf030b02bc7703e5e0a5553971d28dac0238c6fc73b22cbd68d6cd99e88e8861`
was used for initial legacy/browser checks;
`sha256:0f7b830c6e19a52cebb5816f9e14f9ae472d9092743157b8ababd35f09c8d03b`
was the later cleanup/namespace build. The proxy was
`sha256:543d71e7db7b987f1e41dfb0a297efe5f020c141b2dbc39a859fd2473df7bbe2`.
These predate the upstream merge and latest backend corrections. Their successful
runs do not qualify the current combined application.

The following qualification is **historical and superseded**: backend manifest
`90652fcf1c56beace518ce9140ae809dd176310598c437e1c9514c5da525e8e2`
and its 26-unit/six-archive Rust run, Clippy/release and A19 image/runtime checks
qualify the earlier source-version correction matching `5451980`. They do not
qualify later code. The committed EPUB follow-up at
`c9bd0e11c1f6d8ae602546b95cab244c6ece58db` supersedes that block with manifest
`1519753e3b2ba671b9a7ddb5698c54ab6bd1d2b52ec64878156d4afa11a67142`,
38 Rust tests and the full `c4ecab94711e3c8a2972e6cd096be9e17238f8e618398cd1eb30c7e9e42f63f9`
image qualification recorded under the EPUB follow-up below. Later corrections
have their own source/image/result bindings; final acceptance belongs to the
actual PR head and is recorded in its description.

## CodeRabbit dispositions

Severity is the original published severity. “Historical pass” describes the
bounded source/image scenario recorded below. Each qualification applies only
to its recorded inputs; current-head acceptance is recorded in the PR description.
A resolved review thread and a passing check are separate evidence.

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
| D-02 | Warning | Historical `23ba22e` coverage was 100.00% across 56 functions and 12 files. Review 5688495119 on `1ece1df` reports **68.99% below 80%**, 129 functions across 24 files and four unsupported skips. | Current warning remains unsatisfied until the next hosted measurement. Useful lifecycle/ownership contracts were added to complex party, player and harness functions; the threshold is unchanged. The service does not disclose the exact missing-function/file list, so Python-only counts or arithmetic estimates are not a coverage pass. |
| [PR2-DOC-01](https://github.com/ColterD/Frayd/pull/2#discussion_r4396630072) | Minor | Explicit oracle now requires successful proxy-facing liveness/library/progress before, during and after the blocked probe, followed by scan completion. Historical harness rerun exercised all phases. | Documentation and historical verification gap closed; separate from CR-02's Major code severity. |
| [PR2-DOC-02](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5677483709) | Trivial | Explicit oracle preserves cached page names, page-index archive lookup, IDs, metadata and progress after upgrade/rescan. Populated legacy startup refreshed two rows; explicit rescan returned HTTP 202 then idle. All 19 HTTP/eight DB assertions passed after each. | Historical pass for unchanged archives; NEXT-01 covers archive replacement. |
| [FRESH-01](https://github.com/ColterD/Frayd/pull/2#discussion_r4396816530) | Minor | Old cleanup missed a real stopped labelled container. Corrected all-state listing rejects it; removal restores clean checks. Red regression followed by 14 passing Python tests and lint. | Hosted thread resolved/outdated, marked addressed in `3b7a5f5`; historical runtime cleanup passed. |
| [FRESH-02](https://github.com/ColterD/Frayd/pull/2#discussion_r4396816549) | Minor | Two failed jobs originally left four registry rows. Corrected tests require zero per-job row/directory growth across repeat failure while retaining neighbors/legacy artifacts; historical 24 Rust tests and runtime passed. | Hosted thread resolved/outdated, marked addressed in `3b7a5f5`. Later publication-failure risk is separate below. |
| [NEXT-01](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5677830358) | Minor, outside diff | Valid: persisted comic pages can outlive source mtime/size changes. Required oracle replaces an archive, rescans, verifies added page listing/index bytes while retaining ID/progress/other metadata. | Corrected in the final 28-test suite and real combined-image HTTP replacement test. Seconds-granularity mtime plus size does not detect replacements preserving both values. |
| [NEXT-D01](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5677830358) | Minor, unverified tool diagnostic | Report requested “Ubuntu-hosted” wording but explicitly could not confirm its finding. Ledger uses **“hosted run on Ubuntu”** and records the actual historical workflow outcome below. | Wording clarified; diagnostic validity is not claimed. |
| [FULL-01](https://github.com/ColterD/Frayd/pull/2#discussion_r4397136686) | Minor, performance | Reconciliation probes lack indexes for `media(transcode_path)` and `transcode_artifacts(owner_media_id)`. Verify backfill, conflict and cleanup query plans on several thousand converted rows. | Both idempotent indexes are added. Bundled SQLite 3.45.0 plans switch the correlated scans to covering-index searches on a 3,000-source fixture; schema/semantic checks and all 29 Rust tests pass. Thread was marked addressed and resolved on `b2a8504`; plan and retention proof remain recorded. |
| [DOC-COUNT](https://github.com/ColterD/Frayd/pull/2#discussion_r4397295290) | Minor, documentation | Delayed full review `5678313526` on `b2a8504` found a stale 18-entry section heading while the table contained 19 entries. | The section heading is count-free; this finding is included in the reconciled declared total and table. A one-shot count validation checks the declared total against actual rows. |
| [DOC-ALERT-GATE](https://github.com/ColterD/Frayd/pull/2#discussion_r4397497064) | Minor, documentation | Alert 13 was already disposed, but a remaining-acceptance bullet still named it as pending. | Removed from active gates; the approved disposition, source/parser/recorder rationale and historical state remain recorded. |
| [DOC-QA-SCOPE](https://github.com/ColterD/Frayd/pull/2#discussion_r4397497073) | Minor, conditional future scope | Use neutral QA image names and restore upstream checkout instructions before an upstream merge. | Neutral names adopted. The active fork checkout is deliberately retained for this owner-authorized fork-only draft because upstream lacks these fixes. Restoring the upstream checkout is a future upstream-contribution handoff condition, not an unmet gate in this round. |
| [QA-READY](https://github.com/ColterD/Frayd/pull/2#discussion_r4397497091) | Minor, harness reliability | Readiness must retry raw transport resets/disconnects/timeouts alongside HTTP assertion failures until its bounded deadline. | Explicit OSError retry passes recovery/exhaustion cases across seven error variants; all 16 Python tests and the full default-image integration pass. Product runtime source is unchanged. |
| [DOC-PROXY-CMD](https://github.com/ColterD/Frayd/pull/2#discussion_r4397657061) | Minor, documentation | The proxy build used the neutral tag but the following nginx configuration check retained the old tag. | Both commands now use `frayd-proxy-qa:local`; the exact documented build/configuration-check sequence is verified separately. No runtime source changes. |
| [LEGACY-REFRESH](https://github.com/ColterD/Frayd/pull/2#discussion_r4397774540) | Minor | A legacy conversion reference can suppress scanning a separately indexed genuine file at the referenced path. Keeping the database row is insufficient: replacement size, thumbnails and conversion eligibility must refresh. | Baseline actual scan retained stale size 8 instead of 41 bytes. A provenance-aware ownership predicate now permits indexed legacy targets to refresh; the focused regression preserves ID/progress/metadata, generated-artifact exclusions and unindexed legacy compatibility. All 33 Rust tests, Clippy and release pass; fixed-image real-media runtime refresh and restart persistence pass. |
| [DOC-IMAGE-PROVENANCE](https://github.com/ColterD/Frayd/pull/2#discussion_r4398020230) | Minor, documentation | The legacy-refresh red/green section used short image IDs without source-commit bindings. | Full immutable image digests and verified source-commit bindings are recorded in that section; local receipts distinguish build-time working trees from their later commits. |
| [EPUB-REFRESH](https://github.com/ColterD/Frayd/pull/2#discussion_r4398119571) | Minor | Changed ebook fingerprints retain cached `$.spine`, so a replaced EPUB can expose stale chapter navigation. | Actual scanner/chapter and Docker baselines retain the stale three-entry spine and return 404 for the new first chapter. Ebook-only invalidation and snapshot-guarded field-only lazy-cache publication fix the Rust regression; all 38 Rust tests and Clippy pass. Fixed-image full runtime qualification passes; hosted acceptance is recorded in the PR body. |
| [DOC-HISTORICAL](https://github.com/ColterD/Frayd/pull/2#discussion_r4398359380) | Minor, documentation | A superseded manifest and 32-test qualification were incorrectly described as current. | The block is explicitly historical and superseded by the committed EPUB follow-up, with each source/image result kept in its own boundary. |
| [COMIC-CACHE-WRITE](https://github.com/ColterD/Frayd/pull/2#discussion_r4398359394) | Minor | A refused optional comic page-cache write turns a successfully parsed archive into HTTP 404. | A valid archive plus a refused SQLite cache write reproduces HTTP 404. The minimal inline result handling now serves computed pages on write errors, while a successful zero-row update still returns None/404 without cache mutation. Both controls, all 40 Rust tests and fixed-image full runtime qualification pass; current-head hosted acceptance is recorded in the PR description. |
| [HIGHLIGHT-RANGE](https://github.com/ColterD/Frayd/pull/2#discussion_r4401439202) | Minor | A selection crossing text nodes could be saved but not painted; trimming text also shifted whitespace ranges. | Unsupported ranges are rejected before POST with clear feedback; supported ranges preserve raw text and offsets. Six focused controls and all 43 client tests pass locally; corrected-image/browser acceptance remains pending. |
| [BACKUP-MODE](https://github.com/ColterD/Frayd/pull/2#discussion_r4401439211) | Minor | Copying the backup block under a permissive umask did not enforce private archive/directory modes. | The block sets umask 077, creates a new owned mode-700 directory, verifies mode 600 on the archive and refuses existing/symlink/relative targets. Synthetic permissive-umask checks pass; archive/restore mechanics retain their separate same-image scope. |
| [STALE-RESERVATION](https://github.com/ColterD/Frayd/pull/2#discussion_r4401439268) | Major | Killed conversions and cleanup timeouts can leave pending registry pairs and large partial outputs without a reachable retry. | Real killed-worker and five-second DB-lock/rescan baseline reproduced. Lifecycle/lease recovery passes the focused controls and all 51 Rust tests locally; Linux converter liveness is verified separately. Completed, unknown, active, referenced and uncertain artifacts remain protected. The exact bf3cb589/858879bf immutable-image build and HTTP suite pass; latest-head CI/review and held manual browser acceptance are recorded in the PR description. |
| [BACKUP-CHECKSUM](https://github.com/ColterD/Frayd/pull/2#discussion_r4402135393) | Minor | The guide printed a checksum but did not persist it or enforce verification before extraction. | The private sidecar is stored beside the archive and the restore block exits before extraction on failure. Intact/relocated, corrupted and missing-checksum gate controls pass. The exact updated blocks also pass one same-image Docker round trip; a corrupted copy exits before extraction and leaves the destination empty of files. Current-head hosted acceptance remains in the PR description. |
| [HIGHLIGHT-DUPLICATE](https://github.com/ColterD/Frayd/pull/2#discussion_r4402386926) | Unspecified | Selecting an already-saved highlight sends another POST because the cached duplicate check runs after the request. | A guard checks kind, chapter and CFI before POST. Repeated-save and preloaded-save baselines fail with excess requests; five focused controls and all 47 client tests pass after correction. Real-browser acceptance retains its separate gate. |
| [DOC-COMPLETED-WORK](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5685026287) | Minor, unverified wording suggestion | The historical-checkpoint sentence used an unclear compound modifier. | It now says “work completed later.” LanguageTool is unavailable locally; no LanguageTool pass is claimed. |
| [DOC-CURRENT-HEAD](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5685026287) | Minor, unverified wording suggestion | The documentation index used awkward current-head acceptance wording. | It now says “Hosted acceptance for the current head.” |
| [DOC-MOVIE-ACTION](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5685026287) | Minor, unverified wording suggestion | The movie action repeated the observation-column wording. | The action now says “confirm decoded playback”; the recorded observations and held acceptance remain unchanged. |
| [DOC-MEDIA-BACKUP](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5685026287) | Minor, unverified wording suggestion | The separately mounted media instruction repeated “separately.” | It now identifies mounted media as a separate data source. The named-volume restore proof does not cover external media. |
| [REAL-DOM-COVERAGE](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5685026287) | Trivial, unverified coverage suggestion | Mocked Range/TreeWalker and Plyr behavior cannot prove actual DOM selection offsets or source replacement. | Nine real-DOM cases pass using the production generated iframe and actual vendored Plyr: exact selection/save/paint/reload, rejection cases and active hooks/follower seek after two media replacements. The supplied browser version/hash and source hashes are recorded. Synthetic player events do not prove decoded playback or two-client synchronization; interactive latest-image acceptance remains held. |
| [COPY-PRIVATE-MEDIA](https://github.com/ColterD/Frayd/pull/2#discussion_r4402829094) | Minor | The volume-copy example can retain ownership incompatible with UID 10001 while preserving private modes, leaving the copied media unreadable. | The Bash-guarded tar stream extracts as UID/GID 10001 into a new directory. Private 0700/0600 ownership/readability, scan and exact HTTP media bytes pass on the qualified image; repeat-target, invalid-name, missing-source and failed-producer controls fail safely. No chown capability is required; all owned resources are removed. |
| [LEDGER-TABLE](https://github.com/ColterD/Frayd/pull/2#discussion_r4402829103) | Minor | Blank lines end the disposition table, leaving later findings outside it. | GitHub GFM rendered only 24 of 39 entries as table rows before correction. Removing the internal blank lines restores one table; GitHub GFM rendering includes all dispositions, with row counts tied to each recorded ledger source. |
| [COPY-SYMLINK](https://github.com/ColterD/Frayd/pull/2#discussion_r4402974897) | Minor | Tar preserves symbolic links that the scanner skips, so a copied link can leave the expected media absent. | The old guide accepts a link and indexes zero media. The corrected preflight rejects links, FIFO, symlinked source roots (including trailing slashes) and traversal errors before creating the destination. The private regular-file scan/exact-byte path still passes. Source stability and non-atomicity remain explicit. |
| [PARTY-HOST-ECHO](https://github.com/ColterD/Frayd/pull/2#discussion_r4403256417) | Major | The first host state echo reloads an already-active Plyr stream and seeks to zero, resetting local saved resume. | A host-only preserve option recognizes the exact configured stream before either ID/drift branch, including stale same-media IDs, only when video was already active with its modal open. Inactive cached videos reactivate through the normal path. Guests retain authoritative position/play/pause synchronization. Red controls reproduce 203 to zero before/after metadata; 67 Node and 18 actual-DOM cases pass locally. New-image and media acceptance remain required. |
| [DNS-CLEANUP](https://github.com/ColterD/Frayd/pull/2#discussion_r4403256467) | Minor | Unbounded cleanup commands can hang, and inventory errors can replace the original scenario failure. | All cleanup attempts have five-second bounds. Existing scenario errors survive diagnostic/removal/inventory failures; cleanup remains fatal after an otherwise successful scenario. Three real fake-Docker controls and all 20 Python tests pass. The earlier immutable-image DNS regression passes; current client changes need a new app image. |
| [AST-DNS-LOGS](https://github.com/ColterD/Frayd/pull/2#discussion_r4403256467) | Tool diagnostic, CWE-78 | The AST tool claims incoming-request command data at the fixture's Docker logs call. | Not actionable on the supported call path: the container name is a fixed suffix of a generated hex project name, passed as one argv token. No incoming request supplies it and no shell interprets it. Static triage only; no suppression or timeout-as-security claim. |
| [AST-DNS-REMOVE](https://github.com/ColterD/Frayd/pull/2#discussion_r4403256467) | Tool diagnostic, CWE-78 | The AST tool makes the same claim at container removal. | Not actionable: removal iterates only the fixture's generated fixed-suffix names through an argv list. Static source/caller evidence supports this disposition; no command-injection fix or suppression is claimed. |
| [AST-DNS-NETWORK](https://github.com/ColterD/Frayd/pull/2#discussion_r4403256467) | Tool diagnostic, CWE-78 | The AST tool makes the same claim at network removal. | Not actionable: the network argument is the locally generated fixed-prefix hex project name, never request data, and execution uses an argv list without a shell. Static triage only; the independent cleanup bounds address reliability. |
| [NATIVE-ORDER](https://github.com/ColterD/Frayd/pull/2#discussion_r4404067848) | Major | A fixed delay did not guarantee retired-media metadata before requested metadata; one hosted test failed despite correctly resuming playback. | The test holds requested bytes until trusted retired metadata is observed, then releases them. Cache is disabled before navigation in its own fresh headless profile; held/released counts prove the gate is exercised. Missing-event and aborted-browser cleanup are bounded. The attached GitHub Actions diagnostic is supporting evidence for this same requirement, counted once. |
| [EXTERNAL-NATIVE](https://github.com/ColterD/Frayd/pull/2#discussion_r4404939482) | Minor | Party transitions from YouTube to native audio/video leave the external player running; native video can retain the YouTube modal class. | Four baseline cases reproduce the missing pause. The native branch transfers playback ownership before pausing YouTube, then removes the external modal class. Unit and host/guest DOM controls cover fresh/retained audio/video. The YouTube API is mocked; actual external playback and new-image qualification remain separate. |
| [PARTY-TIME](https://github.com/ColterD/Frayd/pull/2#discussion_r4404939496) | Minor | Concurrent updates can acquire the state lock out of captured-time order, regress updated_at and count elapsed playback twice. | The 120,110,120,130 sequence reproduces position 50 instead of 40. Clamping the timestamp under the state lock preserves a monotonic reference. All 53 Rust tests, Clippy and release compilation pass; warnings in unchanged code remain. Stale/equal pause, play, seek, load and skip controls preserve explicit positions and queue actions. |
| [BROWSER-PORT](https://github.com/ColterD/Frayd/pull/2#discussion_r4404939505) | Minor | An existing but empty or incomplete DevToolsActivePort file is treated as an immediate invalid-port failure. | The bounded loop waits for a newline-terminated numeric port in range. Empty, malformed, out-of-range and numeric-partial controls reproduce the old failure and pass after correction. Browser exit and the 15-second discovery deadline remain enforced; the numeric-partial control uses a fetch stub without network requests. |
| [NODE-RUNTIME](https://github.com/ColterD/Frayd/pull/2#discussion_r4404939542) | Minor | The runner requires global WebSocket but CI and development prerequisites leave Node unspecified. | CI selects its existing Node 22.23.4 toolcache executable and verifies its exact version; missing/wrong entries fail before writing PATH. Local checks require Node 22+ and global WebSocket before resource creation. The proposed setup-node action was rejected after dependency-advisory checks; no action dependency was added. |
| [PROFILE-EQUALS](https://github.com/ColterD/Frayd/pull/2#discussion_r4404939555) | Minor | Splitting --user-data-dir on every equals sign truncates an owned profile path containing an equals sign. | Removing the exact known prefix preserves every later equals sign. A real child-writer control uses an equals-containing profile inside a uniquely owned parent and verifies the full write path and process-group/profile cleanup. No guessed outside path is removed. |
| [YOUTUBE-READY](https://github.com/ColterD/Frayd/pull/2#discussion_r4405132483) | Minor | A YouTube player object exists before its methods are ready, so native handoff can throw and a late ready callback can start external playback over native media. | Per-player readiness and current-player/ownership checks prevent unready calls and stale native restarts. Focused baselines reproduce unready transitions, retired callbacks and dropped latest same-video state. Readiness now applies the latest paused position; 73 client tests and the shared 105 Node / 29 DOM controls pass. YouTube API behavior is mocked. |
| [BROWSER-DISCOVERY](https://github.com/ColterD/Frayd/pull/2#discussion_r4405132488) | Major | A complete port file can appear before the owned HTTP target-list endpoint responds; the one-shot three-second request fails before any browser case. | The shared startup deadline covers port, target-list, socket and bootstrap operations. Bounded discovery retry controls cover transient connection/service failures, empty lists, no response and browser exit; unexpected/multiple/nonblank/foreign targets remain fatal. The shared 105 Node / 29 browser run passes, including real fetch/WebSocket/RPC after an injected initial transport failure. Real owned HTTP nonresponse and early-exit controls remain bounded and clean up successfully. |
| [DOC-PROFILE-RETENTION](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5688495119) | Minor, outside diff | The guide promises profile deletion even when the owned browser group cannot be stopped safely. | The guide now states that a failed shutdown records cleanup errors and may retain the profile. Deletion requires confirmed owned-group shutdown. |

Full review [5688243622](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5688243622) selected 41 files and filtered one SVG and one MP4. It published the five Minor requirements above. The formal review contains no additional nitpick, outside-diff or tool section; the shared comment warns that some details were omitted to fit its size limit. Published findings are tracked here without claiming disclosure of every internal warning.

The `d835f40` full review selected 30 files and excluded one deleted SVG. The earlier
`3b7a5f5` review selected five files and skipped `tests/test_integration.py` as
similar to previous changes. Selected-file lists are not the undisclosed exact
coverage-file list, and selection does not prove every line was analyzed. The original review
contained eight actionable items across seven inline records, two nested nitpicks
and two diagnostics; CR-07/08 share a comment but are separate requirements.


## Summary-only risks and separate security work

These summary observations and security records are separate from the formal entries above:

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

## Historical qualification and hygiene

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

## Historical commands and acceptance gates

The commands and gates in this section preserve an earlier qualification checkpoint.
Use [Development](../development.md#canonical-checks) for maintained commands;
check the actual PR head for current acceptance.

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

At this historical checkpoint the client/Python counts were 22/16 passing respectively, exit 0; the
extra Python cases cover readiness transport recovery and bounded exhaustion.
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
docker build --progress plain -t frayd-qa:local .
docker build -f Dockerfile.proxy -t frayd-proxy-qa:local .
docker run --rm --network none --read-only --tmpfs /tmp --entrypoint nginx frayd-proxy-qa:local -t
python3 scripts/check-build-context.py
python3 -u scripts/integration.py
python3 scripts/audit-dependencies.py
```

These commands passed historically, but must run again after the new source freeze.
The context check exports an actual image with synthetic `.env` sentinels. The
audit queries current OSV; the historical 209-package clean result and exact
block-buffer 0.10.4 publisher-range exception do not predict future advisories.
CI repeats syntax/client/Python/lint/Compose/build/context/integration/audit checks.

Acceptance checks and remote record at that historical checkpoint:

1. Final acceptance requires hosted checks, CodeQL and a completed CodeRabbit review,
   including documentation coverage >=80%. The automatic incremental run on
   `9f3b700` was skipped before analysis; its findings/coverage are unavailable.
   The displayed 100% result belongs to `3b7a5f5`, not the skipped or final head.
   Later full-review publications are linked above and the current result is in
   the PR body. Review conversations stay
   open until their complete acceptance oracles and fresh-review gate pass.
2. Retain draft status until owner acceptance. Deployment, production restart,
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
all hosted workflow jobs pass. GitHub marks alerts 11-12 fixed. Alert 13 still flags
the literal option terminator and effective argument controls; it was dismissed
as false positive with explicit owner approval, recorded separately
from the source-fixed alerts. The PR body records the approved full rationale
and current GitHub state. These are separate from the four
EPUB findings already fixed on fork main. No query/code suppression was added;
alert 13 received the separately approved false-positive disposition.

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

## Pre-index immutable-image qualification

The pre-index application is
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

## Full-review follow-up

The full review on `67bf71a` did not repeat the original 18 entries. It reported
one new Minor index issue, tracked as FULL-01 above, and no nitpicks or tool
warnings. Its passing title check still described the former title as too broad;
the PR title will be made specific to scan responsiveness and transcode isolation
with explicit fork-draft scope. The index thread was subsequently marked
addressed and resolved on `b2a8504`, bringing that snapshot to ten resolved
threads; the delayed documentation-count comment is tracked separately.

## Indexed reconciliation follow-up

The FULL-01 regression reproduced with bundled SQLite 3.45.0 on 3,000 converted
originals, 3,005 media rows, 6,001 registry records and one protected progress row.
Exact production SQL showed correlated `SCAN x` during backfill, `SCAN other` for
conflict/ownership probes, and a registry owner scan during cleanup; the focused
index regression failed before the schema change.

`idx_media_transcode_path` and `idx_transcode_artifacts_owner` are non-unique,
idempotent indexes on the two requested columns. With them, all four query plans
use the relevant covering-index searches. The populated legacy schema was applied
twice; backfill added one legacy record, one conflict was reported, 6,001 owned
paths were found, one unreferenced artifact row was pruned and four protected
media/progress records survived. Repeating reconciliation retained final counts
of 3,004 media and 6,002 registry rows. Production Rust and all four captured SQL
queries are unchanged; the Rust change is the regression test only.

All 29 Rust tests, Clippy, release build and whitespace checks pass. These are
query-plan and semantic observations, not a universal scan-latency benchmark.
The post-index immutable build/runtime and next full review are separate gates
whose latest results are recorded in the PR body. The index thread is resolved
and marked addressed on `b2a8504`. The chosen title now names scan responsiveness,
transcode isolation and fork-draft scope, addressing the qualitative title feedback.

## Validated summary observation: converted-source freshness

The zero-actionable `b2a8504` follow-up still noted stale converted media in its
summary. This was a real defect. Actual scan/stream tests served old converted
bytes after a known source replacement. Controlled in-flight jobs also published
a stale path after the source or recorded database version changed. Each baseline
failed before correction; unchanged and initial unknown-legacy controls passed.

Rescan now clears the playback pointer only after a detectable change in known
prior timestamp/size fields. Jobs compare both actual file version and the
database snapshot under the publication write transaction; source changes during
a database-lock wait are also rejected. Rejected new jobs use exact generated-job
cleanup. IDs, progress, metadata and genuine files stay intact. Initial NULL
fingerprints retain compatibility references whose original version is unverified.
Historical completed files/provenance remain recorded; there is no automatic
reaper or storage bound, and no hostile same-UID filesystem-race guarantee.

All 32 Rust tests pass with Clippy/release checks. The repeatable Docker runner
also proves the actual codec path: historical immutable `73df40ae` exited 1
because it returned stale converted bytes after a 7,804→20,144-byte source change
and a controlled five-second timestamp advance. Fixed application image
`sha256:a19b1132c02e958e498f1cff4611c07ee55198c1ac5c3707acb5c7d02287abe3`
exited 0 on the same scenario: old bytes were rejected, a new UUID conversion
contained H264/AAC, exact new stream bytes survived restart, and media ID 2,
progress 0.25, source hash and directory names stayed correct. Both red and green
projects were independently checked for zero remaining containers, networks and
volumes. The earlier read-only/source-preservation checks run before deliberate
fixture replacement and remain intact.

The new harness is included in the existing CI integration command. Its changes
and the source-version fix require final-head CI and review acceptance; prior
zero-comment results are not relabeled as a review of this new code.

The delayed full review `5678313526` on `b2a8504` published a documentation-count
comment after the initial bounded review inventory closed. It is retained as
DOC-COUNT above. The count-free disposition heading avoids repeating an embedded
number; the declared total and actual table rows are checked together. This
metadata-only correction does not change any runtime/build verification input.

## Final documentation and harness dispositions

The `5451980` whole-PR review did not repeat the converted-source freshness risk.
It raised three Minor items, all tracked above. Alert 13 is absent from active
acceptance gates; its approved false-positive rationale and history are retained.
QA image defaults, CI build tags and repeatable commands use neutral names. The
README checkout continues to target the actual fork branch under the owner's
explicit fork-only work order. Current upstream `540b87a` lacks this PR's cache
and scanner fixes, so substituting its default branch now would reproduce a
different program. Restore upstream installation instructions when preparing an
actual upstream contribution; that future condition does not block this fork
draft. Readiness transport retry is covered by focused red/green unit cases and
the disposable integration run, while all 32 Rust runtime tests remain unchanged.

The final readiness/default-name follow-up is verified: the old catch failed
eight raw transport-error subcases, and the corrected helper passes seven error
variants across recovery and exhaustion while retaining its 20-second deadline
and last-cause diagnostic. Python 16/16 and configured Ruff pass. Neutral aliases
were verified absent before being assigned to the exact qualified application
and proxy IDs; no unknown image was overwritten. The integration command without
image flags passes, including both conversion generations and restart/progress
persistence. All 31 image, nine verification and 20 backend bindings match, and
all disposable containers, networks and volumes are removed. No runtime rebuild
is required for these harness/documentation-only changes.

## Indexed genuine legacy target follow-up

Full review `5678944739` found an additional distinction between row retention
and scan eligibility. A legacy playback reference could name a separately indexed
genuine source: the generated-only deletion guard preserved its row, but the
pre-walk ownership set skipped its refresh. The actual scanner regression retained
stored size 8 after replacement with 41 bytes, then passed after limiting this
exclusion for `legacy_reference` provenance when a media path already exists.
Generated provenance and unindexed legacy playback targets retain their existing
exclusion behavior. IDs, progress, metadata and source bytes stay intact; a
subsequent scan publishes the cached thumbnail path.

All 33 Rust tests (27 unit and six archive tests), locked Clippy and release build
pass, with existing style warnings retained. The scale-query oracle is byte-identical
to the production ownership query and confirms an indexed `media.path` lookup;
the eligible genuine legacy target changes its expected excluded-path count from
6,001 to 6,000 without changing preserved rows or user references. Runtime scanner,
main and schema/index code are unchanged. Normal Docker build image `7d77ce2c`
passes its embedded tests, Clippy and release build.

The real-media Docker regression fails against historical A19: after replacing
a genuine indexed file referenced by legacy playback, the database retains
11,833 bytes instead of the actual 31,526. Fixed image `7d77ce2c` passes the same
scenario: stored size and mtime refresh, IDs/titles/device progress 42 stay intact,
source hashes and names remain correct, and the legacy playback reference survives.
Restart and rescan preserve this result. The existing scan-phase, proxy IP-change,
conversion and replacement tests also pass. A first fixture attempt exposed a
cleanup-order gap when switching from a named volume to a host bind; its failed
cleanup receipt is retained, only its exact task-owned volume was removed, and
the corrected harness tears down the initial scenario before switching storage.
Repeated baseline and fixed runs complete with zero owned resources remaining.
The same integration command runs in CI; final-head CI and full review remain
acceptance gates.

### Exact artifacts for the legacy-refresh comparison

Both runs use the integration harness at source commit
`a634a5f4e8c29bca28b854d3ae128fd9039ce660` and the same immutable proxy
`sha256:543d71e7db7b987f1e41dfb0a297efe5f020c141b2dbc39a859fd2473df7bbe2`.
These are locally retained QA images; the identifiers do not claim registry publication.

| Run | Full application image digest | Matching source commit | Observed result |
|---|---|---|---|
| Baseline | `sha256:a19b1132c02e958e498f1cff4611c07ee55198c1ac5c3707acb5c7d02287abe3` | `54519804a27f5139b7ee54c10b604e898af355d9` | Exit 1: stored size 11,833 versus actual 31,526 after rescan. |
| Fixed | `sha256:7d77ce2ccd7967fad3625bb8fa172d6378152607bcb4992e2649934cc0097a5a` | `a634a5f4e8c29bca28b854d3ae128fd9039ce660` | Exit 0: stored size 31,526, refreshed mtime, preserved IDs/progress/source bytes and successful restart persistence. |

All 31 image inputs in each retained build manifest were verified against the
corresponding Git commit objects. The baseline was built from frozen working-tree
inputs while checkout HEAD was `b2a850443f400748e25fc2f3a46661075eb9e0e4`;
those inputs subsequently landed in the matching `5451980` commit above. The fixed
image likewise used frozen inputs while HEAD was `9a22ad2`, subsequently committed
as `a634a5f`. Neither build-time HEAD alone is the image source attribution.
The fixed backend manifest is
`25eb54aca0e82de95484872e0f14e940c9904036f849ffee83dda860b9b1f4b3`;
the baseline backend manifest is
`90652fcf1c56beace518ce9140ae809dd176310598c437e1c9514c5da525e8e2`.
Local receipts retain commands, build manifests, runtime logs, exact fixture
observations and cleanup checks; the full digest/source/result bindings above
make this public ledger independent of the local receipt paths.

## EPUB replacement coherence follow-up

Review `5679402211` identified a separate EPUB cache: replacing an ebook changed
its file fingerprint without invalidating `meta_json.$.spine`. Both the actual
Rust scanner/chapter regression and disposable Docker HTTP fixture reproduce
the failure. The fixture warms three chapter paths, preserves them through an
unchanged scan, then replaces the archive with two chapters using different paths.
The baseline retains the old three-entry spine and returns HTTP 404 for the new
first chapter. Unrelated title/progress/metadata and source bytes are checked
separately from cache invalidation.

The failing runtime artifact is
`sha256:7d77ce2ccd7967fad3625bb8fa172d6378152607bcb4992e2649934cc0097a5a`,
whose 31 image inputs match source commit
`a634a5f4e8c29bca28b854d3ae128fd9039ce660`. The baseline command uses the
unchanged qualified proxy
`sha256:543d71e7db7b987f1e41dfb0a297efe5f020c141b2dbc39a859fd2473df7bbe2`.
Its expected regression failure is retained; independent checks confirm zero
owned containers, networks or volumes.

The fixed source is committed before its image build as
`c9bd0e11c1f6d8ae602546b95cab244c6ece58db`, including the exact integration
harness used for both runs. All 38 Rust tests (32 unit and six archive tests) and
locked Clippy pass with existing style warnings. Five focused EPUB cases cover
replacement, unchanged cached order, missing archives, optional cache-write
refusal, and source-version/path publication guards with preservation of concurrent
metadata. Ebook fingerprint changes remove only `$.spine`; comic/manga `$.pages`
invalidation stays intact. Lazy cache computation moves outside the application
database mutex and publication merges only the spine field under the captured
database source snapshot. A snapshot mismatch cannot publish the old cache.
Optional database cache-write failures retain the previously successful chapter
read behavior, while archive failures remain HTTP 404. The fresh-parsing spine-list
endpoint is unchanged. These controls do not claim atomicity against arbitrary
concurrent filesystem writers or detection of replacements preserving both size
and seconds-granularity mtime.

The fixed normal-build artifact is
`sha256:c4ecab94711e3c8a2972e6cd096be9e17238f8e618398cd1eb30c7e9e42f63f9`,
built from the already-committed source and harness
`c9bd0e11c1f6d8ae602546b95cab244c6ece58db`. Its 31 image, nine verification
and 20 backend inputs match that revision. Its binary SHA-256 is
`960022fa84ff3b2148bd731f2656911e9c8fc1dea24a9307faa52081de1002a7`;
the backend manifest is
`1519753e3b2ba671b9a7ddb5698c54ab6bd1d2b52ec64878156d4afa11a67142`.
The pinned Docker build passes all 38 Rust tests, Clippy and release compilation.
Both baseline and fixed runtime commands use the harness committed at `c9bd0e1`
and the full proxy digest above; only the application image argument differs.

The fixed full Docker runner exits 0. Its EPUB fixture changes from 1,168 to
1,654 bytes with mtime advanced five seconds: the warmed old three-entry spine
is absent immediately after rescan, before any chapter request. Fresh listing
contains the two renamed chapters, chunks 0 and 1 return exact new XHTML, and
removed index 2 returns HTTP 404. Only the spine metadata changes; media ID 3,
title, progress 1 with `updated_at=12346`, custom metadata, the unrelated pages
field and source hashes remain intact. Restart preserves the new reading order
and exact bytes. The prior conversion, source-replacement, legacy-reference and
proxy/scan-phase scenarios also pass. Final-head CI and full review remain gates;
the prior successful reviews are not relabeled as review of this source.

## Optional comic page-cache write follow-up

Review `5679720492` identified a compatibility regression in optional cache
publication: a SQLite write error discarded an already computed archive page
list, making a valid page return HTTP 404. The baseline Rust handler regression
uses a trigger that refuses cache writes and observes that 404. The bounded
inline result handling now distinguishes success with one updated row, success
with zero updated rows and a database error. A database error logs the held cache
write and returns the computed page list; zero rows still reject stale
publication with None/HTTP 404. The guarded publication SQL and fingerprint
predicate are byte-identical to the baseline. No helper, schema or dependency
was added.

Committed source and harness
`b852a7f87bc1ad9e3921d7b98202ccc2b719b70c` pass all 40 Rust tests (34 unit
and six archive tests) and locked Clippy with existing style warnings. The
positive trigger-refusal test verifies exact page bytes and list content without
cache mutation; a separate ignored-update trigger verifies the zero-row guard.
The backend manifest is
`9fce609cd7c132f48bae1f5fff5b03c6874e52bee1eaed4e0b7a557f4ef42133`.
Fixed-image full runtime qualification passes as recorded below.

The baseline runtime image is
`sha256:c4ecab94711e3c8a2972e6cd096be9e17238f8e618398cd1eb30c7e9e42f63f9`,
matching committed source `c9bd0e11c1f6d8ae602546b95cab244c6ece58db`.
With the fixture's targeted SQLite refusal trigger active,
`/api/comic/4/pages` returns HTTP 404 instead of 200; the runner exits 1 at that
expected assertion after all earlier scenarios pass. Its task-owned containers,
networks and volumes are independently confirmed absent.

The fixed image is
`sha256:cd880c88f8c97dbebe771a3303455c714b3a03dde732cc8474634575b7f4cf58`,
built after committing source and harness as
`b852a7f87bc1ad9e3921d7b98202ccc2b719b70c`. All 31 image, nine verification
and 20 backend inputs match that commit and freeze. Binary SHA-256 is
`e33f03a60d996d5227110d1447759a5b015c123951e5910a1d70072450831e87`.
Both runtime commands use the harness at `b852a7f` and immutable proxy
`sha256:543d71e7db7b987f1e41dfb0a297efe5f020c141b2dbc39a859fd2473df7bbe2`;
only the application image argument differs. The normal Docker build passes
40 Rust tests, Clippy and release compilation.

The fixed full runner exits 0 with the refusal trigger still installed. Comic
ID 4 returns HTTP 200 with page list `['001.png']` and exact original PNG bytes
for page 0; page 1 correctly returns HTTP 404. SQLite metadata still has no pages
cache, and the row's ID/title/custom metadata, device progress 0 and source bytes
remain unchanged. Restart repeats the same successful reads without a cache write.
The real EPUB replacement, legacy-reference refresh, conversion/source replacement,
scan-phase and proxy recovery scenarios all pass in the same run. Both baseline
and fixed fixtures have zero owned containers, networks or volumes after cleanup.
The configured CI integration command includes this case. Final-head CI and full
review remain acceptance gates for the combined change.
