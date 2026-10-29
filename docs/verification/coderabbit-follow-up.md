# CodeRabbit follow-up verification

[Documentation index](../README.md) · [Operations](../operations.md) · [Development and canonical checks](../development.md)

This is the historical verification ledger, not a setup guide or a current-head
acceptance report. Use [Development](../development.md#canonical-checks) for the
maintained command list. Every result below applies only to its recorded source,
image and scenario. Earlier pending gates describe their historical checkpoint;
they do not reopen work completed later. Hosted acceptance for the current head belongs to
the [draft PR description](https://github.com/ColterD/Frayd/pull/2). Draft status
remains independent of deployment acceptance. Later owner-authorized local rollout
and its remaining interactive-acceptance gate are recorded in the
[verified local rollout](pr-2-changelog.md#verified-local-rollout).

Navigation: [source boundaries](#source-and-delivery-boundaries),
[dispositions](#coderabbit-dispositions),
[historical commands and gates](#historical-commands-and-acceptance-gates),
[EPUB follow-up](#epub-replacement-coherence-follow-up),
[comic cache follow-up](#optional-comic-page-cache-write-follow-up).

Updated 2026-10-29 UTC for [fork draft PR #2](https://github.com/ColterD/Frayd/pull/2).
The disposition table tracks original and subsequent CodeRabbit findings,
outside-diff comments, nitpicks and published tool diagnostics. Summary-only risks
and GitHub CodeQL alerts have separate sections. Current measured counts and
head-specific acceptance are recorded in the PR body.
PR #2 remains draft and unmerged. This ledger records historical verification;
deployment authority and qualification are scoped to the later rollout record above.

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
| D-02 | Warning | Historical scores were 100% on `23ba22e`, 68.99% on `1ece1df`, and 77.69% on `acf719f`. Review 5689101822 on `7fa4c3e` reports **81.54% >=80%**, 130 functions across 24 files and two unsupported skips. | Qualified for `7fa4c3e`; later-head measurements are recorded in the PR body. Useful state/ownership contracts were added without changing the threshold. The exact missing-function/file list is undisclosed; local Python-only counts are not the hosted oracle. |
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
| [YOUTUBE-READY](https://github.com/ColterD/Frayd/pull/2#discussion_r4405132483) | Minor | A YouTube player object exists before its methods are ready, so native handoff can throw and a callback that arrives after native handoff can start external playback over native media. | Per-player readiness and current-player/ownership checks prevent unready external calls and stale external restarts. Local native entrypoints now share the ownership transition so they cannot overwrite the prior external kind before cleanup. Focused baselines reproduce unready transitions, retired callbacks and dropped latest same-video state. Readiness now applies the latest paused position; 87 client tests and the shared 119 Node / 30 DOM controls pass. YouTube API behavior is mocked. |
| [BROWSER-DISCOVERY](https://github.com/ColterD/Frayd/pull/2#discussion_r4405132488) | Major | A complete port file can appear before the owned HTTP target-list endpoint responds; the one-shot three-second request fails before any browser case. | The shared startup deadline covers port, target-list, socket and bootstrap operations. Bounded discovery retry controls cover transient connection/service failures, empty lists, no response and browser exit; unexpected/multiple/nonblank/foreign targets remain fatal. The shared 105 Node / 29 browser run passes, including real fetch/WebSocket/RPC after an injected initial transport failure. Real owned HTTP nonresponse and early-exit controls remain bounded and clean up successfully. |
| [DOC-PROFILE-RETENTION](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5688495119) | Minor, outside diff | The guide promises profile deletion even when the owned browser group cannot be stopped safely. | The guide now states that a failed shutdown records cleanup errors and may retain the profile. The profile is deleted only after the runner confirms that the process group it owns has stopped. |
| [NATIVE-EXTERNAL](https://github.com/ColterD/Frayd/pull/2#discussion_r4405365960) | Minor | Switching from native audio/video to an external party URL can leave native playback running underneath YouTube. | Two focused baselines reproduce native playback continuing. Ownership now switches before pausing both native players, preventing stale pause broadcasts. All 87 client tests and the shared 119 Node / 30 browser controls pass; external YouTube playback remains mocked. |
| [DOC-CALLBACK-TIME](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5688787475) | Minor, unverified wording suggestion | A tool objects to the compound modifier describing a delayed readiness callback. | The text now says “a callback that arrives after native handoff,” stating the event order directly. No unavailable language-tool pass is claimed. |
| [DOC-GROUP-SHUTDOWN](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5688787475) | Minor, unverified wording suggestion | A tool objects to the cleanup sentence's compound modifier. | The text now says the runner confirms that the process group it owns has stopped before deleting the profile. This preserves the failure-retention contract; no language-tool pass is claimed. |
| [QUEUED-NATIVE-PAUSE](https://github.com/ColterD/Frayd/pull/2#discussion_r4405615196) | Minor | The fixture dispatches pause synchronously although native media queues it, masking a stale host pause after a rapid external-to-native return. | Decoded native playback reproduces the queued ordering. Actual current-element/state guards ignore a trusted stale pause delivered while playing and publish one legitimate paused event. Queue-faithful audio controls and all 119 Node / 30 DOM cases pass; the prior synchronous-only fixture was insufficient. |
| [PENDING-NATIVE-LOAD](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5689101822) | Minor, outside diff | A native metadata callback can run after external handoff and restart native playback that was already paused. | External handoff invalidates native loads; delayed callbacks require the current generation, kind, element and exact stream. Local progress promises and metadata callbacks cannot resume retired media, even after a return to native playback. Ten focused baseline failures become green; all 119 Node / 30 DOM cases pass. |
| [CACHED-VIDEO-ACTIVATION](https://github.com/ColterD/Frayd/pull/2#discussion_r4405902934) | Minor | Following a cached movie after closing its modal or starting local audio can leave the video hidden and audio playing because activation depends on source replacement. | Both baselines fail before correction. Handoff, presentation and current item/progress identity now refresh on every nonpreserved follow; only source assignment remains conditional. Both DOM paths activate with zero video-source assignments; the shared 133 Node / 30 DOM run passes. |
| [LEDGER-DECLARATION](https://github.com/ColterD/Frayd/pull/2#discussion_r4405902986) | Minor | The introduction still declared 48 records while the rendered table had grown to 61. | The introduction now describes the table without duplicating its mutable count. The PR body and GFM receipt record the measured count for each source revision; historical source-specific counts remain qualified. |
| [PARTY-START-OWNER](https://github.com/ColterD/Frayd/pull/2#discussion_r4406015132) | Minor | Overlapping Start responses can replace the first host socket and end the party they just created. | One pending request owns Start through HTTP and JSON completion. Leave and Join invalidate stale responses; failures permit retry without releasing a newer request. Staggered-response baselines fail; all 18 focused party tests and the shared 133 Node / 30 DOM run pass. |
| [YOUTUBE-HOST-SENDER](https://github.com/ColterD/Frayd/pull/2#discussion_r4406015143) | Major | A reused player retains the guest session's host flag, preventing Play/Pause publication after hosting the same video. | The inline anchor is party.js, but the captured callback is in app.js. Callbacks and one rebound seek poller use the current host sender, retaining identity/readiness/ownership guards. Guest-to-host reuse publishes play, pause and seek with one player; guest/departed sessions send nothing. The shared 133 Node / 30 DOM run passes; the external adapter is mocked. |
| [NODE-PROVISIONING](https://github.com/ColterD/Frayd/pull/2#discussion_r4406484592) | Major | CI requires an exact preinstalled Node cache path that a runner image may not contain. | A valid exact-version cache remains the fast path; a missing/invalid entry uses the official archive with pinned SHA-256 before extraction and an exact runtime guard. Twelve controlled checks pass, including actual Node download/WebSocket, no-network cache reuse and corrupt-archive rejection. Prior hosted passes do not reproduce missing-cache failure; no new action dependency was added. |
| [EFFECTIVE-STREAM-IDENTITY](https://github.com/ColterD/Frayd/pull/2#discussion_r4406484598) | Major | A local media selection can replace the element source while the cached last-party ID remains unchanged; refollow can misattribute another movie's position. | Both Node and real DOM baselines reproduce media 9 receiving media 10's position and audio retaining stream 10. Source change and ownership guards now share the effective stream comparison. All 93 client tests, 135 combined Node tests and 32 DOM cases pass. Actually matching child sources retain zero source reassignment; the separate progress-ownership correction below prevents stale local trackers from writing another stream’s position. |
| [REPLACED-SOURCE-RETIREMENT](https://github.com/ColterD/Frayd/pull/2#discussion_r4406484606) | Major | Clearing a known replaced source's playback pointer leaves the completed generated pair outside recovery. | The pointer-clear transaction retires only the exact registered, inactive, unreferenced prior pair after containment and lease checks. Three real recovery cycles remove six stale rows and three directories while preserving the historical neighbor. Five scanner tests cover 17 modes; the normal combined Rust suite passes 57 tests. Unknown, referenced, active and unrelated history retain protection. |
| [LINUX-RETIREMENT-ORACLE](https://github.com/ColterD/Frayd/pull/2#discussion_r4406654190) | Minor | A Linux-only lease/recovery test is compiled and asserted on other targets, including a Unix-only symlink call on Windows. | The test now compiles only on Linux, matching the production lease/recovery contract. Its focused Linux execution passes; production scanner bytes before the test module are unchanged. No non-Linux build is claimed. |
| [PROGRESS-SAVE-WAIT](https://github.com/ColterD/Frayd/pull/2#discussion_r4406654195) | Minor | The effective-stream regression uses a fixed delay although pause-driven progress saving is asynchronous. | The test waits for the requested effective stream before pausing, then waits for the video progress request near 24 seconds. Audio has no progress-save wait. The updated oracle passes 135 Node and 32 DOM cases, including six decoded controls; the old source still fails both stream waits and the Node wrong-progress/source assertions. |
| [DECODED-FIXTURE-SCOPE](https://github.com/ColterD/Frayd/pull/2#discussion_r4406719666) | Minor | The development guide still describes every player event as synthetic after actual native decoding controls were added. | The guide separates synthetic IDs 9/10/11 from owned H.264 IDs 20/21/22, names readiness/width/positive-frame oracles and retains physical-audio, other-codec and distinct-client timing limits for that fixture. Separate actual YouTube API/peer observations stay in their own checkpoint. |
| [HOST-REGISTRATION-PROOF](https://github.com/ColterD/Frayd/pull/2#discussion_r4406874506) | Major, security | A disclosed device key can claim the first host slot or replace the connected host, then control playback or end the room. | Creation now issues a private UUID capability bound to creator identity and session generation. One predicate checks it before upgrade and again before membership changes; malformed, ambiguous, wrong, stale and wrong-identity proof is rejected. Only the public WebSocket protocol is negotiated. Causal real-socket attacks become 18 passing rejection/legitimate scenarios; focused Rust 11 and panel 23 tests pass. Full package/image gates remain separate. |
| [IMPORT-OWNER-ACCESS](https://github.com/ColterD/Frayd/pull/2#discussion_r4406874499) | Minor | Applying a private umask to group-readable source modes can produce unreadable imported files and untraversable directories. | Source tar archive headers normalize owner access and remove group/other permissions before app-UID extraction. Other-UID mode-0040 files and nested group-only directories reproduce the old failure; corrected imports are private, readable and exact, without changing source ownership/modes/bytes. Existing private-file and five refusal controls pass under UID 10001 with dropped capabilities. |
| [LINUX-TRANSCODE-ORACLES](https://github.com/ColterD/Frayd/pull/2#discussion_r4406874512) | Minor | The transcode test module uses Unix APIs and Linux-only cleanup behavior but compiles on unsupported targets. | The whole test module now uses cfg(all(test, target_os = "linux")), matching its production recovery contract. All 18 focused Linux tests pass. Only the Linux target is installed; no non-Linux compilation is claimed. |
| [BOUNDED-PAUSE-ORACLE](https://github.com/ColterD/Frayd/pull/2#discussion_r4407150991) | Minor | A fixture waits indefinitely for pause even when playback has not started and pause emits no event. | It now waits for playback, uses a one-time listener and bounds pause observation with the existing deadline, removing the listener in finally. Missing-playback and missing-event controls terminate around four seconds with no retained listeners. Actual DOM 32/32 and all six decoded controls pass; audio wrong-ID and video progress oracles remain intact. |
| [ACTIVE-GUEST-CONTINUITY](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5691012247) | Major, originally unverified; runtime validated | A member reuses an advertised active guest key, evicts the victim and submits requests under its label. | A causal real-socket baseline confirms eviction and impersonation. The shared role predicate rejects guest keys with open outbound receivers before upgrade and atomic registration. Fourteen focused Rust tests and ten real-socket scenarios pass, covering retained victim/observer traffic, preflight races, closed-receiver rejoin, stale cleanup and valid-proof host replacement. This protects active slots; first-slot/inactive identity authentication and global creation are outside the claim. |
| [COMPLETED-CACHE-DOCS](https://github.com/ColterD/Frayd/pull/2#discussion_r4407203979) | Minor | Operator guidance says completed cache files always remain until manual cleanup despite known-replacement retirement. | Both retention statements now explain that a qualified prior completed pair can retire and later be deleted by recovery. Pending/retiring recovery is distinguished from retained unrelated or uncertain history. Operators are told to preserve original media and app data rather than rely on generated cache for recovery. |
| [SCAN-WALK-WARNINGS](https://github.com/ColterD/Frayd/pull/2#discussion_r4407392792) | Minor | Walk and manga-probe errors are discarded, silently omitting unreadable paths while the scan completes. | Explicit walk, directory-open, entry and file-type error branches warn with operation/path and continue. A real mode-000 subtree under UID 1000 reproduces the empty-warning baseline; the corrected scan warns while indexing an accessible item, retaining old rows/progress/bytes and skipping links. Six focused scanner tests pass. Root-builder execution drops child UID/GID before creating fixtures and asserts actual PermissionDenied; full-image execution remains a separate gate. |
| [REPLACED-VIDEO-DURATION](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5691427197) | Minor, outside diff | Replacing an indexed video updates its fingerprint while keeping the old duration. | Real one-second to three-second replacement reproduces stored duration one on the baseline and three after correction. Known movie/episode changes invalidate the old duration; successful probes publish under a write transaction only when current file and captured database fingerprints match. Failed known probes leave NULL, unchanged values remain, and failed unknown-version probes retain unqualified legacy values. Identity/progress and three controlled stale-publication cases pass; seven focused scanner tests pass. |
| [IMPORT-PATH-COMPONENTS](https://github.com/ColterD/Frayd/pull/2#discussion_r4407536057) | Minor | The import recipe checks only the final source component, so a symlinked ancestor can reach find, tar and destination creation. | Preflight checks each component in traversal order before any copy command. The baseline accepts five ancestor-link variants; the correction rejects seven path variants before command markers. Four real private/group-only/space-path copies and nine refusal controls pass under app UID/GID 10001 with no capabilities. Fifteen source entries retain their metadata/bytes/targets, and disposable resources are removed. The trusted source must remain unchanged during this non-atomic recipe. |
| [BUILD-TEST-MEDIA-TOOLS](https://github.com/ColterD/Frayd/pull/2#discussion_r4407612193) | Major | The real duration regression requires ffmpeg/ffprobe, but the Rust build stage installs neither tool. | The normal clean build reproduces 58 passing unit tests and one missing-executable failure before archive tests, Clippy or release. Installing the existing distro FFmpeg package in the build stage enables the unchanged normal command: diagnostic 59 unit plus six archive tests, Clippy and release pass, with the exact real-media duration oracle also passing. Runtime-stage instructions stay unchanged. Signed-package provenance and existing runtime-equivalent tool hashes are recorded; open distro advisories remain a separate image-hardening risk. |
| [RESTORE-CREATE-COMMAND](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5691679200) | Minor, outside diff | A literal ellipsis makes the destination Compose command nonexecutable. | The destination instructions give an executable distinct-project command with pull and build disabled. An isolated same-image configuration executes that exact command successfully; both containers remain created, not running, with PID zero and only empty data scaffolding. All newly owned resources are removed. This proves creation only, not a full recovery round trip. |
| [RUST-COUNT-PROVENANCE](https://github.com/ColterD/Frayd/pull/2#discussion_r4407771981) | Minor | An unscoped Rust count drifts as implementation and test targets change. | Browser acceptance now binds 59 server unit plus six archive tests to pre-upstream revision 7e3a7ee and the normal Docker build-stage cargo test --locked command. The fresh image receipt and executed Cargo output verify 65, independently of the review's alternative count. Ongoing integration results are kept separate. |
| [PARTY-CREATOR-IDENTITY](https://github.com/ColterD/Frayd/pull/2#discussion_r4407771988) | Minor | Creation binds raw body identity while WebSocket admission prioritizes a valid Bearer identity, so a hand-crafted client's valid proof is rejected. | Creation and WebSocket admission share canonical identity resolution before session mutation. A valid Bearer with a different device body reproduces WebSocket 403 before correction and 101 afterward. Device-only clients and private creator-proof enforcement remain supported; rejected creation preserves the current room. The review identifies consistency failure, not an authorization bypass. |
| [TOOLBAR-ACCOUNT-GENERATION](https://github.com/ColterD/Frayd/pull/2#discussion_r4408655080) | Minor | An older toolbar settings response can cross an account-generation change. | Account lifetime is separate from Settings navigation. Six stale start/status/library success and failure controls reject publication after logout/new account, while valid scan completion survives opening and closing Settings. The shared eight-case baseline fails before correction and passes afterward; all 175 Node tests pass. Synthetic browser fixtures do not qualify actual browser interaction. |
| [NATIVE-CHECK-HELPER](https://github.com/ColterD/Frayd/pull/2#discussion_r4408655086) | Major | The native guide's reproducible checks run helper tests after Clippy without explicitly building the helper executable. | The guide explicitly builds frayd-native into the same target directory after Clippy and before helper tests. A clean target using cached dependencies reproduces Clippy exit 0 with no helper executable, then build exit 0 with the executable present. The native follow-up separately passes 20 tests, including actual relocated Linux x86_64 lifecycle; these local results do not qualify Windows or new macOS execution. |
| [NATIVE-EXTRACT-MODES](https://github.com/ColterD/Frayd/pull/2#discussion_r4408655097) | Minor | Native archive extraction instructions must preserve recorded permission modes. | The packaged README and native guide use tar -xpzf. With umask 077, ordinary extraction reproduces changed manifest modes and install refusal (exit 2) without changing the destination tree; preserved-mode extraction retains index mode 0644 and script mode 0755, then installs and removes successfully while retaining the data marker. The focused real-archive test passes; other tar implementations/platforms remain separate qualification. |
| [FAKE-INTERVAL-MONOTONIC](https://github.com/ColterD/Frayd/pull/2#discussion_r4408655103) | Minor | Allocating a fake interval from the live Map size can reuse an active identifier after another timer is cleared. | A monotonic interval identifier counter preserves live callbacks after an earlier timer is cleared. The focused allocation regression fails before correction and passes afterward within the shared eight-case red/green run; all 175 Node tests pass. This is fixture behavior, not a production timing measurement. |
| [NATIVE-PROCESS-DEADLINES](https://github.com/ColterD/Frayd/pull/2#discussion_r4408655109) | Minor | Native qualification subprocess calls need bounded completion deadlines. | Finite fixture commands use a 60-second default deadline in a private process group, with bounded cleanup. A one-second timeout regression reproduces a surviving descendant holding its lease with parent-only cleanup; group cleanup makes that lease immediately acquirable and preserves TimeoutExpired. The focused control passes. Foreground servers retain separate bounded readiness/wait/finally supervision. All 20 native tests pass, including the actual relocated Linux x86_64 lifecycle; this does not establish a new macOS or Windows run. |
| [UPFRONT-SPLIT-ASSERTION](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5692815839) | Trivial | Review-body marker 86b5a4223b65abd7791abfcc requests an upfront assertion that the expected source split exists. | The harness source is checked for its expected marker before splitting or starting the client VM. A missing-marker negative control triggers the explicit assertion before VM execution; all 175 Node tests and syntax checks for the three changed JavaScript files pass. |
| [NATIVE-SUBPROCESS-TOOL](https://github.com/ColterD/Frayd/pull/2#discussion_r4408655109) | Error, tool diagnostic | Embedded ast-grep 0.50.0 rule subprocess-from-request reports CWE-78 in the native test subprocess path. | False positive for request taint: the unittest helper receives literal test commands, owned fixture paths and explicitly supplied helper configuration. No HTTP handler or request reaches its argv; urllib is an outbound fixture client. Execution uses an argv list with shell=False. Source inspection validates this boundary; timeout liveness is the separate valid requirement above. No suppression, product security rewrite or alert-state change is implied. |
| [TOOLBAR-STATUS-RETRY](https://github.com/ColterD/Frayd/pull/2#discussion_r4408760457) | Minor | A transient toolbar status error stops polling permanently, and HTTP 409 cannot reattach to an active scan. | Both 202 and 409 start account-scoped polling at the existing 1500 ms cadence. At most one status/refresh request runs at a time; a successful scanning status resets failures, while three consecutive failures stop monitoring and restore manual retry. Five new controls fail on the prior source and pass after correction; all 13 focused controls and 180 Node tests pass. Stale-account publication guards remain in force; actual browser qualification is separate. |
| [NATIVE-EXTRACTION-DIRECTORY](https://github.com/ColterD/Frayd/pull/2#discussion_r4408760464) | Minor | The native extraction command selects a destination directory that the guide has not created. | The guide creates a new mode-0700 extraction directory before tar -xpzf. The missing-directory baseline exits 2; the exact documented sequence exits 0, followed by successful install/removal with the data marker retained. The real local extraction uses paths containing spaces. No new cross-platform execution is inferred. |
| [TIMEOUT-READY-HANDSHAKE](https://github.com/ColterD/Frayd/pull/2#discussion_r4408760473) | Minor | The native timeout oracle starts its deadline before the descendant is ready and holding its lease. | A ten-second bounded handshake observes descendant readiness and proves its lease is held before starting the one-second communicate timeout. A two-second startup reproduces the old false readiness assertion and passes with the corrected test; the lease is acquirable after group cleanup. The general bounded runner is unchanged. All 20 native tests pass, including the actual relocated Linux x86_64 lifecycle; no fresh macOS or Windows execution is claimed. |
| [NATIVE-EXAMPLE-LOOPBACK](https://github.com/ColterD/Frayd/pull/2#discussion_r4408814566) | Major | The runnable native cargo example inherits the all-interface bind default despite its surrounding loopback guidance. | The command explicitly supplies FRAYD_BIND=127.0.0.1 while preserving its library value and cargo invocation. Static checks confirm every runnable cargo example in the seven guides supplies loopback, shell syntax passes and the source bind_address parser preserves that literal. No native server was launched against operator environment or data. Docker's internal bind remains unchanged. |
| [DOCKER-OPERATION-DEADLINES](https://github.com/ColterD/Frayd/pull/2#discussion_r4408814572) | Minor | Integration Docker commands share a deadline that does not reflect operation-specific completion bounds. | Explicit budgets distinguish metadata (20 seconds), lifecycle/cleanup (60), Compose startup and synthetic ffmpeg conversion (120), and other exec calls (30); unsupported operations fail closed. A fake Docker CLI with a descendant retaining pipes and a lease proves timeout cleanup kills the private group, drains output within ten seconds and releases the lease before return. Diagnostics omit private argv/stdin/output. The original follow-up passed 13 focused tests and selected Ruff rules; the later shared-caller correction below passes 14. Image/runtime acceptance remains source-bound separately. |
| [INTEGRATION-SUBPROCESS-TOOL](https://github.com/ColterD/Frayd/pull/2#discussion_r4408814572) | Error, tool diagnostic | Embedded ast-grep 0.50.0 subprocess-from-request reports CWE-78 for integration subprocess execution. | False positive for incoming request taint: this is a CLI runner using argparse image values, generated Compose JSON and internal argv lists. Its request() function is an outbound urllib client; no incoming HTTP handler supplies process arguments. Popen explicitly uses shell=False, and container shell snippets are fixture constants. Source inspection supports this boundary; no rule suppression, tool rerun or alert-state change is implied. |
| [SCANNER-PREFLIGHT-WARNINGS](https://github.com/ColterD/Frayd/pull/2#discussion_r4408814580) | Minor | Scanner validation and directory-creation early returns omit path/reason diagnostics. | Structured warnings identify operation, cache kind, path, source root and reason before cache-validation or thumbnail-directory-creation returns; creation failure also records the OS error. The old source fails for missing diagnostics. The corrected focused test exercises transcode overlap, thumbnail overlap and a regular-file mkdir blocker, while preserving database rows/progress, registry and fixture bytes without creating directories. The focused test, 12 scanner tests, 91 server tests plus six archive tests and Clippy pass; five pre-existing comic/EPUB Clippy warnings remain. New-image runtime proof is separate. |
| [DOCKER-SHARED-CALLER-COMPATIBILITY](https://github.com/ColterD/Frayd/pull/2#discussion_r4408909023) | Major | The operation allowlist omits build/create/export/removal calls made by check-build-context.py through the shared Docker helper, so the normal context check fails before creating its probe. | Repository-wide caller inventory identifies the context checker and its five required operations. Explicit budgets add context build (600 seconds), create (30), export (60), container removal (60) and image removal (60), preserving unknown-operation refusal and existing cleanup ownership. All 14 focused tests, selected Ruff rules and the actual unchanged context checker pass. An instrumented real run confirms its exact owned container/image are removed and no temporary paths remain. This helper-only correction does not qualify a new application image. |
| [NATIVE-REMOVE-OFFLINE-DATA](https://github.com/ColterD/Frayd/pull/2#discussion_r4409012790) | Minor | Native program removal is blocked when its separately owned data directory or data marker is unavailable. | Prefix/payload validation is separate from the start-only data receipt check. Moved or deleted data and missing or malformed markers reproduce removal refusal with the old helper; the corrected helper refuses start but removes only owned program payload without recreating data or changing surviving snapshots. Modified/unknown payload, uncertain liveness, data redirects and overlap still refuse removal unchanged. Four focused tests, all 22 native tests including the actual relocated Linux x86_64 HTTP lifecycle, helper build and Clippy with warnings denied pass. The new helper/package is bound by its working-tree hashes and archive checksum; the base commit alone does not identify it. Fresh macOS qualification remains separate from historical platform receipts. |
| [PARTY-PREADMISSION-REJECTION](https://github.com/ColterD/Frayd/pull/2#discussion_r4409233649) | Minor | An upgrade rejection can fire error then close before any session message; the close renderer replaces the visible rejection with “Party ended.” | The socket lifecycle preserves the rejection until a session message establishes admission. The focused party controls pass. On exact `a4027b8` image `670d14d9…`, a real unknown-code join returned HTTP 404, then error/close left “Party not found or rejected” visible with Join available and no sender. The fixture receipt below binds that browser result; other rejection causes remain untested in a real browser. |
| [BROWSER-BUDGET-DOC](https://github.com/ColterD/Frayd/pull/2#discussion_r4409233657) | Minor | This page described the historical 15-second discovery budget in present tense and implied one shared 60-second guard. | The [browser guide](browser-acceptance.md#hosted-browser-startup-budget) separates historical 15-second observations from the 30-second startup and separate 60-second case-phase budget in `a4027b8`. Local timeout controls and 49 actual-browser cases pass; the hosted browser workflow has its own result gate. |
| [RETAINED-JOB-RESERVATION](https://github.com/ColterD/Frayd/pull/2#discussion_r4409233663) | Major | On a host without qualified recovery, repeated scans can reserve new jobs while an uncertain generated artifact remains, including after source replacement. | An SQLite immediate transaction checks an unfinished generated job for the media ID before creating a new namespace; it does not require a matching source path. Focused main and archive controls pass locally. Non-Linux runtime and new-image qualification remain pending. |
| [IMPORT-RECIPE-TEMP-ROOT](https://github.com/ColterD/Frayd/pull/2#discussion_r4409233674) | Major | A symlinked `TMPDIR` makes the recipe reject the fixture ancestor before testing the requested source. | The fixture root is resolved before test paths are built. Two focused symlinked-`TMPDIR` controls pass locally with GNU tools; no macOS run is inferred. |
| [IMPORT-RECIPE-GNU-TOOLS](https://github.com/ColterD/Frayd/pull/2#discussion_r4409233678) | Major | macOS BSD `find` and `tar` can be selected for a recipe that requires GNU options. | The test selects verified GNU-compatible tools, prefers `gfind`/`gtar`, and explicitly skips when neither qualifies. GNU and preferred-tool controls pass; BSD-only controls skip rather than claiming a recipe pass. Fresh macOS qualification remains pending. |
| [DOCSTRING-COVERAGE-FBB](https://github.com/ColterD/Frayd/pull/2#issuecomment-6459955958) | Warning, separate shared pre-merge check | The issue comment updated 2026-10-29T06:12:34Z reports 64.59% against the required 80%, across 257 functions in 32 files with 18 unsupported skips. It has no per-run or head marker; the earlier unbound 80.63% observation is historical. | This historical shared report is below threshold. A later exact-head check must establish whether the added documentation clears the gate. It is not a sixth inline finding from review 5693571831. |
| [SCAN-REFUSAL-STATUS](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5693817473) | Minor, outside diff | With an unavailable configured root or cache, early `run_scan` refusal leaves status with no failure. The client reads idle as “Scan complete.” | The server publishes path-free scan failure status, and the client avoids completion feedback or refresh on refusal. Baseline root/cache and UI failures became passing focused controls; 94 server and 186 Node tests, syntax checks and Clippy passed locally. The [782 checkpoint](acceptance-checkpoints.json) and its hosted job links bind the later integrated source. The first focused lane held browser regression because Chrome/Chromium was unavailable; the 782 browser fixture ran separately. |
| [SCAN-STATUS-ALL-ROOTS](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5693945755) | Nitpick | The unavailable-root privacy assertion checked only `/roots/b` although the fixture configures `/roots/a` and `/roots/b`; no disclosure was observed. | The disposable HTTP assertion now rejects either configured root in the raw scan-status response. It runs with the existing refusal, preservation and successful-recovery controls; exact-head execution is recorded in the PR description. This changes verification, not server behavior. |
| [EXTERNAL-PROGRESS-OWNER](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5694082034) | Minor, outside diff | A pending native progress write can continue after external playback takes ownership. | The corrected owner guard retires that write. The [client controls](../../tests/client.test.cjs) reproduced three baseline failures, then passed focused callbacks and positive host-close pause controls. The final combined working tree passes 25 focused cases and 210 total Node tests. Actual Chromium passes 49 cases with cleanup, including neighboring decoder/transitions, but has no exact same-stream external-progress oracle. New image and hosted acceptance remain separate. |
| [CLOSED-NATIVE-METADATA](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5694082034) | Minor, outside diff | Closing a native player can leave a pending metadata callback with authority to resume it. | Closing invalidates pending native load authority while preserving legitimate progress save and one host pause. The same [client controls](../../tests/client.test.cjs) pass 25 focused VM cases and 210 total Node tests on the final combined working tree. Chromium passes 49 cases with cleanup, including decoded close/reopen, but does not isolate every close-before-response callback. New-image qualification remains separate. |
| [SCAN-STATUS-TRANSPORT-RETRY](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5694082034) | Minor, outside diff | A transient scan-status request failure can stop the integration wait before the scan outcome is known. | The [integration polling tests](../../tests/test_integration.py) reproduce two escaping transport errors and an overrun; four focused cases now pass. Status GET retries only bounded transport failures within 30 seconds, capping request and sleep time to the remaining deadline; HTTP denial and invalid JSON fail fast and scan POST is not replayed. The full Python suite passes 36 tests with two native skips, Ruff and compilation pass. Disposable Docker HTTP integration for this change is untested. |
| [PUBLIC-QA-CHECKPOINT](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5694082034) | Minor, outside diff | Verification pages linked ignored local `.codex/evidence` receipts that GitHub readers could not open. | The [public checkpoint](acceptance-checkpoints.json) summarizes non-security source-bound results, artifact IDs and hosted job links; raw operational receipts remain local. The workflow checks JSON syntax. Its contents distinguish direct 4e execution from 782 results carried by input equality, without publishing private fixture or session data. |
| NATIVE-PROGRESS-EVENT-BINDING | Local follow-up to the playback findings | A Plyr wrapper does not deliver native pause/ended events to `.onpause`/`.onended` properties, so a decoded movie paused before the periodic save can lose progress. Native audio progress assignments also overwrite the UI pause handler, leaving the play glyph stale. | [Client controls](../../tests/client.test.cjs) and [actual-browser cases](../../tests/browser-regression.cjs) reproduce the classes: movie pause red 48/1 then green 49/0 with a progress POST near 0.314 seconds; decoded audio glyph red 45/4 then green 49/0 at 320/768/1024/1440 CSS pixels. One exact listener registration/disposal path preserves UI handlers. The final combined working tree passes 25 focused and 210 total Node tests. Natural ended-event and listener cleanup are VM tested; no separate decoded natural-end browser oracle or new-image qualification is claimed. |
| [SETTINGS-CLOSE-SCAN-REFRESH](https://github.com/ColterD/Frayd/pull/2#discussion_r4410020506) | Minor | Closing Settings while its scan continues can cancel the completion refresh, leaving the dashboard stale. | Scan monitoring and completion refresh survive closing Settings for the current account; an older monitor cannot clear its successor. A pinned baseline has six failing cases and one passing control; the later combined working tree passes 230 Node tests. The existing 49-case browser fixture covers adjacent Settings flows, but the exact close-during-scan result uses modeled API responses. Fresh source-bound image and hosted acceptance remain pending. |
| [HOST-AUDIO-ECHO](https://github.com/ColterD/Frayd/pull/2#discussion_r4410020522) | Major | A host can lose a native audio seek when its own party state echo follows a media change, including subsequent progress saves. | The client preserves host position across its own echo. A new combined production-VM regression starts canonical native media 10 at saved **106**, admits an own zero-position echo, advances its isolated clock one second, invokes the registered progress interval and native pause listener, and inspects actual `/api/progress` request bodies. The original `591d0fa` app in an owned temporary tree fails after the echo: position **0**, no tick POST and pause POST **0**. The current app passes: position **106**, tick and pause each POST **106** for media 10, with no sub-106 save. Focused 21/21 passes in five runs and full Node **267/267** passes. Only the new 40-line test is working-tree code after `6187141`; production source is unchanged. VM element/timer/transport boundaries do not establish decoded audio, live peer or hosted execution. Earlier modeled Chrome seek observations and other source-bound fixture results remain separate. |
| [PARTY-PUBLIC-DISPLAY-LABELS](https://github.com/ColterD/Frayd/pull/2#discussion_r4410020578) | Major | The party wire exposed a private member label where a public display label was expected. | [Party source](../../server/src/party.rs) and [client source](../../client/js/party.js) now use public `Host` / `Guest N` labels and a separate `self_label`; canonical authorization identity remains separate. Focused source tests and the shared 224 Node, 97 server plus six archive Rust tests, Clippy and release build pass locally. This is a concise published-finding disposition; fresh image/runtime and hosted acceptance are separate. |
| [BOUND-BUILD-PACKAGE-ACQUISITION](https://github.com/ColterD/Frayd/actions/runs/40838611915/job/123329253160) | Local CI reliability follow-up | A PR job on `591d0fa` was cancelled after 30 minutes while its runtime APT stage had not finished; the same-head [push run](https://github.com/ColterD/Frayd/actions/runs/40838606539) passed. The underlying network cause is unproven. | [Dockerfile](../../Dockerfile) bounds update to 180 seconds and install to 360, each with a ten-second kill grace, 30-second HTTP/HTTPS transport timeout and two retries. Four controlled stalls that previously exceeded a four-second watchdog now exit 124 within the shortened control, leave no child and do not reach the next step. Both exact pinned-base package stages pass in 49.446 and 62.822 seconds; a partial-index control exits 0 in relaxed mode and 100 with strict mode. The [offline package tests](../../tests/test_build_packages.py) run in Python discovery. These checks establish bounded failure, not network recovery or a new full hosted-image pass. |
| [SETTINGS-SCAN-RETRY-AND-409](https://github.com/ColterD/Frayd/pull/2#discussion_r4410649210) | Minor, incremental review 5695479963 | A transient status failure retires a Settings scan observer; HTTP 409 is treated as refusal instead of attaching to the active scan. | Scan-mode status GET retries bounded transport/5xx errors, resets its consecutive-failure count after a valid status, and keeps one account-owned observer for 202/409 without replaying POST. Ingest still stops on failure; deterministic denial and malformed JSON fail fast. A pinned focused baseline fails five of nine controls; the corrected local Node suite passes 242 cases. The changed Settings browser control and final source-bound image/hosted gates remain pending. |
| [BROWSER-DISCOVERY-BODY-CANCEL](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5695479963) | Minor, outside diff | Discovery retries after transient 408/429/5xx can retain response bodies and connections through the startup deadline. | The harness cancels the transient body before retry and keeps one startup deadline. Real owned ephemeral HTTP controls send unfinished 408/429/503 bodies; two baseline controls fail, then focused controls and 17 browser-lifecycle tests pass. The full local Node suite passes 245. Cancellation rejection/timeout are bounded and remain fatal. No completed final-head hosted browser result is claimed. |
| [BROWSER-SHUTDOWN-INTERCEPTION](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5695479963) | Minor, outside diff | Closing the protocol session while the browser process group is still alive can remove network interception before owned shutdown is confirmed. | The harness closes the session only after confirmed group stop and retains interception on uncertain shutdown. An injected shutdown refusal leaves the group alive, session open and profile retained while the runner records failure/nonzero exit status; the live retained session can keep Node alive until the outer CI deadline. The exact owned group is cleaned by the test finally. Confirmed shutdown still closes the session and surfaces profile-removal failure. Focused baseline and corrected lifecycle controls pass as above; actual uncontrolled browser death is untested. |
| [NATIVE-SIGNAL-OWNERSHIP](https://github.com/ColterD/Frayd/pull/2#discussion_r4410759663) | Minor, full review 5695633349 | Native helper termination signals can leave an owned child/process group running. | The active correction is being checked with eight real INT/TERM signal actions across helper/group/child and both readiness states; negative exit-7/SIGKILL controls must retain refusal. A portable run has 22 passes and one archive/source-input skip, so the required final-archive 23-case qualification and current-head hosted/platform results remain pending. Do not infer cleanup from helper exit alone. |
| [RETAINED-UNKNOWN-QUALIFIED](https://github.com/ColterD/Frayd/pull/2#discussion_r4410759671) | Minor, full review 5695633349 | A retained `unknown` generated job can lose its reservation when recovery is otherwise qualified, allowing a second namespace. | The active correction holds an unfinished `unknown` job by media ID independently of source path and recovery qualification. A focused baseline reproduces the duplicate-reservation path; the current working-tree Rust suite reports 98 server plus six archive tests passing. Final committed-source image/runtime, non-Linux and hosted checks remain pending. |
| OWNED-HOST-INITIAL-POSITION | Actual 56 image/browser QA finding and local video control | Party-host native audio with saved position near 64 seconds initially publishes zero to an owned peer before later seeking to the saved position. The same pre-metadata play ordering can affect native video. | The corrected roster/request oracle reaches the real audio path and records the zero-then-64 sequence; a new six-case VM baseline has four causal failures and two passing zero-position controls for pre-metadata video play. The active fix publishes only after owned metadata/resume readiness and guards video pre-play state. Final actual two-peer audio/video repeats, committed image and hosted acceptance remain pending; earlier modeled-party controls do not qualify them. |
| [PARTY-VIDEO-TAKEOVER-PENDING](https://github.com/ColterD/Frayd/pull/2#discussion_r4410945222) | Minor, incremental review 5695879705 | A pending video load flag can survive opening one video, switching to external playback, and later following a party video, suppressing the new host play. | The `2aca11c` correction resets the pending flag only when a party-follow video actually takes ownership, after the same-owner early return retains its state. Four new production PartyUI wire cases and 15 initial-load controls pass together; the Linux image and Node 265 pass. A separate two-context actual server/UI/WebSocket fixture exits 0: actual browser Skip input through CDP yields one host Play wire, two peer playing states after the wire, and both native players playing. Releasing the delayed old progress GET leaves the same video/source, pending false and time advancing. The keyboard fallback was not needed after this core pass; physical audio and owner Vivaldi remain untested. |
| [AWAIT-DUPLICATE-AUDIO-METADATA](https://github.com/ColterD/Frayd/pull/2#discussion_r4410945240) | Minor, incremental review 5695879705 | A duplicate metadata assertion can run before the asynchronous audio handler settles, making the regression nondeterministic. | The `2aca11c` test waits for the handler outcome before asserting duplicate rejection. Focused controls, Node 265 and the Linux image gate pass. This corrects test ordering; it does not establish a separate playback feature or owner-browser acceptance. |
| [PARTIAL-CHILD-MARKER-POLL](https://github.com/ColterD/Frayd/pull/2#discussion_r4411053341) | Minor, full review 5696020097 | The orphan-receipt marker can be empty/partial or disappear after the existence check. The original comment requested both temporary `JSONDecodeError` and `FileNotFoundError`; the earlier JSON-only correction missed the latter race. | The completed test-only catch handles both exceptions under the existing ten-second deadline. Injected disappearance after exists=True raises `FileNotFoundError` before correction and succeeds after; transient invalid JSON still succeeds. Persistent disappearance refuses after 10.030 seconds and 498 retries rather than passing. Current native-harness SHA-256 `9cd1dff2aa7c376b1060579d4109bf1116693ff6402195c2cf2428ec26eeaccc` passes 23 Linux x86_64 archive tests with zero skips, Ruff and Python compilation against the unchanged b755e6f archive SHA-256 `cf5f2ad01a06d2a9837e380bf1a70a0e1102756ae2c7d00ed0bb13dfa0d7704b`. This remains the same finding, not a 129th disposition. The thread was reopened for final commit/CI verification; final-head hosted and Mac status belongs to the PR description. |
| [INSTALLER-BROWSER-CASE-BUDGET](https://github.com/ColterD/Frayd/actions/runs/40857799994/job/123393074707) | Local CI reliability follow-up after `b755e6f` | The PR browser job reached case 48 after 48.103 seconds of its 60-second case phase, leaving 11.897 seconds for a real five-poll installer case that needs about 12 seconds; it timed out with 47 preceding cases passing and no assertion failure. | The test driver begins the isolated own five-poll iframe before independent responsive cases 43–46, preserving its actual production 3-second × four-gap schedule and a separate 15-second completion deadline. A paired local Chrome run changed total case time 55.383 → 48.114 seconds and installer wait 12.279 → 5.017 seconds; all 49 cases pass with cleanup. Focused 26 and full Node 266 pass. This is a test-only schedule correction with one paired timing observation, not a product or general performance guarantee. New committed-head hosted browser acceptance remains pending. |
| [WIRE-ZERO-ELAPSED-TEST-CLOCK](https://github.com/ColterD/Frayd/actions/runs/40859778643/job/123399781440) | Local CI reliability follow-up after `4d3c988` | The PR Node job passed 265/266 tests and failed an audio wire assertion expecting exactly zero after asynchronous settle; the observed projected position was about 0.001 seconds. The [push job](https://github.com/ColterD/Frayd/actions/runs/40859771442/job/123399757611) failed a different new party-video zero assertion with the same near-zero projection. The tests stamp authoritative packets from wall-clock `Date.now()` then await settlement, allowing legitimate elapsed playback time. | The test-only `tests/client.test.cjs` correction supplies an optional clock to the browser VM before production code loads, then stamps authoritative wire packets from that same VM clock. A forced 1 ms tick reproduces both exact-zero failures before the change; after it, focused 20/20 passes in ten separate invocations and full Node 266/266 passes. A separate realm probe keeps host and unrelated VM clocks unchanged; takeover tests explicitly advance one second to retain elapsed-position coverage. No production, browser-driver, native or workflow input changes. Both `4d3c988` hosted Checks remain red; a new committed-head run is required. |
| [NATIVE-RUNTIME-INPUT-GUARD](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5697906709) | Minor, outside diff | Optional NativeRuntime discovery checked only FRAYD_NATIVE_ARCHIVE, then unconditionally read FRAYD_NATIVE_BUILD_SOURCE; archive-only discovery reproduced KeyError before runtime validation. | The guard now requires both inputs. Permanent NativeSourceContract.test_runtime_discovery_requires_both_inputs runs actual child unittest discovery in four environments: neither, archive-only and source-only each exit 0 with one explicit skip; both synthetic inputs admit the runtime test and produce the expected missing-checksum FileNotFoundError, not KeyError or skip. Full required-input Linux native qualification passes 24/24 with zero skips, Ruff and Python compilation exit 0, source SHA-256 `8f12ac8b066a1886c55520c0e63da89ef79c0a59212d8c397db9c4950cf6b23a`, against unchanged b755e6f archive. This is a new finding, separate from the completed marker-race disposition. Prior 12d99b9 CI and fresh Mac 23-per-architecture results are historical; the changed harness requires new committed-head CI/Mac and semantic review verification. |


Browser budget follow-up to `abceebf923841c956d8f69a8353785fbd9118fdd`: the hosted PR attempt exhausted its 15-second startup deadline before any case; the push attempt reached readiness after 13,423 ms, then exhausted the shared 60-second timer during cases. A scaled controlled reproduction confirms that startup consumed case time. The corrected harness gives startup 30 seconds and starts the unchanged 60-second case budget only after setup/navigation acknowledgment. Existing bounded cleanup and per-case oracles remain unchanged. Independent startup/case timeout controls pass; all 183 Node tests and 49 actual-browser cases pass locally, with approximately 204 ms startup, 49,809 ms cases and successful cleanup. Completed/current progress remains separate from the terminal result. This qualifies the recorded harness inputs, not a fresh hosted run or an explanation for the underlying header delay. Native source, package, tests and workflow remain byte-identical to `abceebf9`; earlier native platform executions retain their actual source identities.

Full review [5693571831](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5693571831) is bound to `fbb01af0ce3fa42b698e41a08db03248a383e33c` and was submitted at 2026-10-29T06:10:34Z. Its five inline findings are recorded above. The separate shared [pre-merge coverage comment](https://github.com/ColterD/Frayd/pull/2#issuecomment-6459955958) reports a warning without an exact per-run/head binding. The corrections were committed as `a4027b8220b2b8c2f511ae6675d1696b2639a25b`; the source-bound qualification below replaces the earlier working-tree status for that source.

### `a4027b8` source-bound acceptance checkpoint

The [`a4027b8` structured checkpoint](acceptance-checkpoints.json) retains the exact source, app/proxy/native artifact identities, fresh test counts, runtime results and hosted job links. The local operator receipt also records an initial native attempt that failed because the helper refused an unqualified source-checkout target; the corrected target/build rerun passed all 22 tests. Linux results do not transfer to macOS.

The isolated [browser checkpoint](acceptance-checkpoints.json) binds the same app and proxy images to a 60-step, 28-WebSocket-event actual-browser pass. Unknown code `QZ9X` produced HTTP 404; after error and close, the rejection notice remained visible, Join remained available, and no sender existed. The actual YouTube API retained the same player across guest-to-host rehosting; pause and play reached the peer, whose clock advanced 4.4386 → 6.9473 seconds. The five public sample copies matched their historical hashes before, after and after browser work. The original `/data` bind remained opaque and unchanged; no private database or credential content was read. These are exact-fixture observations, not audible-output, owner Vivaldi, authenticated-account, other-provider or live-deployment proof.

The same structured record preserves HTTP/storage, controlled SERVFAIL recovery, fixture startup/case timings and zero remaining resources for all three owned QA projects. The Chrome fixture uses real DOM/decoded-media controls with modeled APIs; the separate actual YouTube API/peer observation above is a different oracle. These results qualify the named a402 source only.

Incremental review [5693817473](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5693817473) is bound to `a4027b8220b2b8c2f511ae6675d1696b2639a25b` and was submitted at 2026-10-29T06:41:46Z. It publishes the one outside-diff Minor above. Its correction changes server and client behavior after the a402 checkpoint; the a402 image, browser and hosted results remain historical for that source. The focused result does not replace fresh runtime or hosted acceptance for the corrected source.

Hosted CI, macOS architectures and exact-head docstring coverage have separate acceptance gates. Their final run identities and outcomes belong to the [draft PR description](https://github.com/ColterD/Frayd/pull/2); this checkpoint preserves only the exact source and artifacts named here. A later documentation-only commit does not mean these runtime scenarios executed at that later commit.

Full review [5694082034](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5694082034) is bound to `4e5651f40d2fcc3e21782899462e8c7c5f8bc24c` and was submitted at 2026-10-29T07:13:20Z. It published four outside-diff Minor comments, recorded in the table above, including the portability gap in the verification pages. The [public checkpoint](acceptance-checkpoints.json) preserves a402, 782 and 4e non-security QA as distinct source-bound observations. It reports direct 4e execution separately from test and image results carried from 782 by exact input equality; hosted check URLs are status and log references, not local receipt files. The three other findings change client or integration behavior after 4e and require their own focused and later source-bound gates in the PR description.

Full review [5694619803](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5694619803) is bound to `591d0fa7d3a3f201fa59070aa4d568abcc905e18`, submitted 2026-10-29T08:14:11Z. It publishes the three inline findings above. The same-head [PR CI job](https://github.com/ColterD/Frayd/actions/runs/40838611915/job/123329253160) was cancelled at the 30-minute job limit while runtime APT acquisition had not finished; the [push run](https://github.com/ColterD/Frayd/actions/runs/40838606539) completed successfully. That contrast does not identify the underlying network cause. The bounded package correction and focused proofs above apply to later working-tree changes. Settings, audio and public-label corrections likewise require fresh source-bound image, hosted and platform qualification. The older [public checkpoint](acceptance-checkpoints.json) remains historical for a402, 782 and 4e only.


Incremental review [5695479963](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5695479963) is bound to `56a79d33cda563d1a27b9708f44dd9e5b5e1ccdf` and was submitted 2026-10-29T09:28:01Z. It contains one inline Settings retry/409 finding and two outside-diff browser lifecycle findings, recorded above. Local focused corrections are working-tree changes after that head. The original 56 image, hosted Checks and browser result do not transfer to modified inputs without a fresh source-bound run.

Full review [5695633349](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5695633349) is bound to `56a79d33cda563d1a27b9708f44dd9e5b5e1ccdf` and was submitted 2026-10-29T09:40:07Z. It has 55 selected files, two binary assets filtered, and two inline Minor findings above; no outside-diff or nitpick finding was published. Native signal ownership and retained-unknown reservation corrections remain active working-tree work with final-archive and fresh hosted gates pending. The owned browser fixture separately found the initial hosted-audio position issue; the corrected roster predicate is an oracle repair, not a product fix. The original fixture ended held, and no later source or image is called qualified here.


Incremental review [5695879705](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5695879705) is bound to `cdce17c510a1f7d19c260c159116ebff36703bab` and was submitted 2026-10-29T09:58:36Z. It adds the two Minor findings above. The 4-case takeover guard and awaited metadata assertion are later working-tree changes; cd Linux, browser, owned fixture, Mac and hosted results cannot be assigned to those changed inputs. A new committed source and complete checks are required before closing them.


Full review [5696020097](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5696020097) is bound to `cdce17c510a1f7d19c260c159116ebff36703bab`, submitted 2026-10-29T10:10:29Z. It selected 55 files and filtered two binary assets, published the one inline Minor test-poll finding above, and reported no outside-diff or nitpick finding. The `cdce17c` Linux, browser, owned fixture, macOS and hosted results remain source-bound to that commit. The marker-poll correction is later working-tree work requiring a new committed source and archive-backed native test; this review does not qualify the eventual final head.

The [`2aca11c` structured checkpoint](acceptance-checkpoints.json) retains source/artifact identities, fresh Node/Python/native counts, exact-input Rust carry and the pre-commit-equal Chrome fixture result. No fresh Rust compilation is claimed. The later working-tree marker poll red/green and bounded refusal controls reran all 23 native cases against that archive. Its chronological actual-server/UI/WebSocket takeover retains the detailed action and expected/actual observations in [browser acceptance](browser-acceptance.md#2aca11c-browser-checkpoint). The cd hosted Checks, CodeQL and both Mac jobs remain evidence for cd only; this is not remote PR-head or deployment acceptance.


Full review [5693278916](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5693278916) covers `7b695747` through `927f66cc5c9714e4b076b489c83a555f86ab93f7`, with 53 selected files and two filtered assets. Its check completed successfully at 2026-10-29T05:30:28Z and the acknowledgment finished at 05:30:30Z. It publishes one Minor finding above and no additional review-body, nitpick, outside-diff, tool or unverified section. Current coverage is unavailable because shared warnings were omitted. Passing CI, CodeQL and native platform receipts for `927f66cc` remain historical boundaries; the new native helper correction requires fresh source-bound qualification, including each macOS architecture separately.


Automatic incremental review [5693140190](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5693140190) covers `156a8ab1` through `ea6ded21c31d8650d5ede8e028be8a5171e6ede8`, with 11 selected files and five similar-change skips. Its check completed successfully at 2026-10-29T05:11:52Z and publishes the Major shared-caller compatibility finding above. The prior deadline implementation passed its scoped tests but missed a legitimate imported-helper caller; the later inventory and real context execution establish the corrected compatibility boundary. Review completion remains separate from current-head acceptance.


Full review [5693020155](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5693020155) covers `7b695747` through `156a8ab1fca8f6fc8a27a38d34bb230fc91113ef`, with 53 selected files and two filtered assets. Its check completed successfully at 2026-10-29T04:53:21Z and the acknowledgment finished at 04:53:23Z. It adds three inline findings and one separately tracked embedded tool diagnostic above, without review-body nitpicks, outside-diff or unverified sections. Shared warnings were omitted and current coverage is unavailable; no unbound metric is assigned to this review.


Incremental review [5692951781](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5692951781) covers `1a4301cb` through `156a8ab1fca8f6fc8a27a38d34bb230fc91113ef`, with nine selected files and two similar-change skips. It adds three Minor findings above, without review-body nitpicks, outside-diff, tool or unverified sections. Prior source-specific acceptance remains historical; no fresh coverage measurement is inferred. The separate full review on `156a8ab1` completed later as 5693020155, recorded above.


Full review [5692815839](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5692815839) covers `7b695747` through `1a4301cbff32847673841fd56635b1ab813bd66d`, submitted 2026-10-29T04:20:48Z, publication batch 1/1, with 53 selected files and two filtered SVG/MP4 assets. It publishes five inline findings, one Trivial review-body nitpick and one embedded tool diagnostic, tracked separately above. No additional outside-diff or unverified section is published. The coordinator's passing CI, Linux, separate ARM64/Intel macOS and CodeQL receipts for `1a4301cb` remain historical proof for that source; they do not qualify these subsequent corrections. The shared unbound 80.63% docstring observation below supplies no fresh coverage metric for this review.

Full review [5691742517](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5691742517) covers `a7a6dab` through `7e3a7ee`, with 42 selected files and two filtered assets. It adds RUST-COUNT-PROVENANCE and PARTY-CREATOR-IDENTITY without additional nitpick, outside-diff, unverified, duplicate or tool sections. Incremental review [5691679200](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5691679200) separately adds RESTORE-CREATE-COMMAND. The earlier `e0273e0` full request was cancelled when its head changed and supplies no full-review proof. Shared docstring output later reports 80.63% against 80%, 160 functions, 25 files and 15 unsupported skips, but lacks a current run/head marker; it remains a shared-check observation rather than an attributed metric for the integrated source.

Incremental review [5691554757](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5691554757) covers `3e5279d` through `e0273e0`, with five selected files and three similar-change skips. Its BUILD-TEST-MEDIA-TOOLS requirement matches the independently observed missing-tool build failure and the same builder-only correction. The diagnostic execution precedes the corrected commit; final immutable-image and hosted gates remain separate.

Full review [5691464634](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5691464634) covers `a7a6dab` through `3e5279d`, with 41 selected files and two filtered assets. It adds IMPORT-PATH-COMPONENTS, without new nitpick, outside-diff, additional-unverified, duplicate or tool sections. Incremental review [5691427197](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5691427197) separately adds the outside-diff REPLACED-VIDEO-DURATION requirement and reports docstring coverage of 82.05% against 80%, across 156 functions and 24 files with three unsupported skips. The full output omits verbose details and supplies no replacement metric. GitHub Advanced Security's separate annotation for test-only numeric UID logging is not a CodeRabbit requirement; its output-minimization fix is described below.

Full review [5691294988](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5691294988) covers `a7a6dab` through `bfd3aca`, with 41 selected files and two filtered assets. It adds SCAN-WALK-WARNINGS, with no further nitpick, outside-diff, additional-unverified, duplicate or tool section. The preceding incremental review publishes no actionable comments and docstring coverage of 81.82% against 80%, across 154 functions and 24 files with four unsupported skips. The full output omits verbose details and supplies no replacement metric.

Full review [5691074067](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5691074067) covers `a7a6dab` through `34cbefb`, with 41 selected files and two filtered assets. It adds COMPLETED-CACHE-DOCS, without new nitpick, outside-diff, duplicate or tool sections. Incremental review [5691012247](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5691012247) adds BOUNDED-PAUSE-ORACLE and an explicitly unverified active-guest claim; the later controlled reproduction validates that claim. These are two requirements, not duplicate full-review records. Full output omits verbose details and publishes no new docstring metric.

Full review [5690680279](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5690680279) covers `a7a6dab` through `fc14655`, with 41 selected files and two filtered assets. It adds HOST-REGISTRATION-PROOF, IMPORT-OWNER-ACCESS and LINUX-TRANSCODE-ORACLES. No additional nitpick, outside-diff, duplicate or tool section is published. Its full-review metric is absent; the preceding incremental result is 82.43% against 80% across 148 functions and 24 files, with three unsupported skips. The full output retains the verbose-detail omission warning.

Full review [5690486617](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5690486617) covers `a7a6dab` through `5d63ac2`, with 41 selected files and two filtered assets. It adds DECODED-FIXTURE-SCOPE; no further nitpick, outside-diff, duplicate or tool section is published. The preceding incremental review [5690407263](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5690407263) adds LINUX-RETIREMENT-ORACLE and PROGRESS-SAVE-WAIT. Its published coverage is 82.43% against 80%, across 148 functions and 24 files, with four unsupported skips. The full review publishes no new metric and warns that verbose details/warnings were omitted to fit its comment limit; no claim covers undisclosed internal warnings.

Full review [5690203516](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5690203516) covers `a7a6dab` through `0fbbedf`: 41 selected files and two filtered assets. It publishes NODE-PROVISIONING, EFFECTIVE-STREAM-IDENTITY and REPLACED-SOURCE-RETIREMENT, all Major. No additional nitpick, outside-diff or tool section is published. Its execution check succeeds; finding disposition depends on the corrections and verification above. The full review publishes no new docstring metric; the prior incremental score is 80.60% against 80%, with 134 functions in 24 files and two unsupported skips.

Full review [5689608434](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5689608434) covers `a7a6dab` through `776b030`: 41 selected files and the SVG/MP4 filters. Its cached-video comment repeats CACHED-VIDEO-ACTIVATION and is supporting evidence, not another requirement. It adds PARTY-START-OWNER and YOUTUBE-HOST-SENDER. Published coverage is 81.82% against 80%, across 132 functions and 24 files, with 15 unsupported skips. These are that source revision's service results.

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
- **PR CodeQL alert 24, fixture response:** [js/reflected-xss](https://github.com/ColterD/Frayd/security/code-scanning/24) reports query-to-script flow in the loopback browser fixture. The fixture now selects only fixed null/transient/reattach JavaScript literals through an explicit switch; unknown input returns 400 without reflection. Eleven real HTTP controls include hostile and prototype-like inputs. The final local harness passes 49 actual-browser cases and all 182 Node tests, bound to its recorded source hashes. JavaScript analysis 2051279460 for PR head `927f66cc5c9714e4b076b489c83a555f86ab93f7` (merge `dea341875f2552904111e279877348837b82ec41`) reports zero findings, and the alert instance reports fixed without dismissal. No suppression was added. The 49-case browser pass remains bound to its recorded client/harness inputs; later helper changes require their own qualification. That historical stalled-first-response control used a 15-second startup limit and a shared 60-second overall timer. The later budget follow-up separates those phases; the underlying hosted Chrome header delay remains unproven.
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

## Replacement party sessions

An isolated two-client real YouTube run exposed a separate server defect: after
a guest left and created a new party with the same player, the old host's
disconnect broadcast `ended` into that replacement. This is a runtime finding,
not an extra CodeRabbit disposition-table entry.

[Party state](../../server/src/party.rs) now has one transition lock, private
generation tokens and per-socket tokens. Creation retires the old registry and
resets playback atomically. [WebSocket admission](../../server/src/main.rs)
captures and rechecks its generation after identity resolution. Stale
registrations, frames and cleanup cannot mutate a replacement or a newer socket
with the same identity. The current host's Leave still ends the current party.

The real WebSocket baseline fails and the corrected five-scenario harness
passes. At the `5d63ac2` checkpoint, eight focused party tests include the five
existing timeline controls and the normal combined Rust suite passes 57 tests. The server proactively closes
retired sockets, so old wire commands are not asserted as delivered; production
receive-path unit controls prove stale-frame rejection. The rebuilt `5d63ac2` /
`767fe…` runtime passes the actual two-client YouTube callback oracle: A retains
its original guest player while rehosting, B joins the surviving replacement,
and actual A API pause/play produces outbound commands, authoritative peer
state 2/1 and an advancing peer clock. Both players are muted; no physical-audio
or decoded-frame claim is made. Owned contexts, browser/profile and rooms are
cleaned up.

## Progress ownership after party audio

The effective-stream regression also exposed an adjacent wrong-item write: after
local audio item 10 installed its tracker and party playback selected item 9,
pausing the party stream wrote `{media_id:10, position:24}`. This is a separate
runtime finding, not another CodeRabbit disposition-table entry.

`trackProgress` now checks its tracker generation, metadata readiness and the
canonical current native stream before periodic, pause or ended saves. Delayed
retired callbacks cannot clear a successor's interval, including same-ID
reopens. Following party audio retains its existing no-new-tracker policy; it
simply cannot corrupt the prior local item's bookmark.

The focused baseline has one positive control pass and four ownership failures;
the actual DOM baseline has 31 passes and the one wrong-item failure. Corrected
focused checks pass 5/5 and all client tests 97/97. The combined gate including
the host-proof tests passes 144 Node tests and the DOM suite 32/32, including all six decoded-video controls. The audio handoff
plays the requested stream at the requested position without the old-ID save.
Current-item periodic/pause/ended saves remain positive controls. Container
qualification and real external-player observations remain separately bound to
their exact runtime artifacts.

## Host control and public creation

Device/member labels are public identity, not host authorization. The create
response alone returns `host_capability`. The host keeps it only for its socket
upgrade offer; it is not written to a URL, localStorage, party state, queue or
membership messages. Only `frayd-party` is negotiated back to the client.

The capability must match the creator identity and the captured live generation
before registration can remove or add any member. The check is repeated after
identity resolution/upgrade. Valid creator-proof reconnects retain their socket-isolation behavior. Guest
replacement requires the previous receiver to close or cleanup to finish;
stale callbacks cannot remove its successor. Current-host Leave still ends
the room. Missing proof never falls back to identity-only hosting. Updated host
clients are required; legacy no-protocol guest joins remain supported.

Any guest can still create or replace the single household party under the
existing trusted-deployment policy. This correction protects control of an
existing room; it does not introduce exclusive authorization for global party
creation or qualify public hosting. Security reproductions and sanitized
validation are retained through the managed security artifact collection, not
inside repository reports or operator credential files.

## Active guest continuity

Guest device labels remain public trusted-local identifiers. A copied label
must not displace an existing guest whose outbound receiver is open. One shared
role predicate checks this before upgrade and again under the registration
lock, including two admissions that both passed preflight before either joined.

The original real-socket reproduction receives the victim's advertised label,
registers a duplicate, observes the victim close and relays a request as that
victim. The corrected ten-scenario harness preserves victim and observer
traffic, rejects encoded-key/invalid-Bearer collision attempts and proof on
guest keys, and preserves valid creator-proof host replacement. Unit tests
retain live receivers explicitly; a separate closed-receiver control proves
rejoin and stale-frame/cleanup isolation.

This guard deliberately fails closed until the server observes receiver
retirement. Immediate same-key rejoin can temporarily reject. Once a slot is
empty, this scheme does not authenticate ownership of its public device label;
first-slot/inactive identity authentication and exclusive household creation
remain outside scope. Security reproductions and sanitized receipts use the
managed security artifact collection.

## Unreadable scan paths

Scan warnings identify the operation and the available failed path. Walk errors
use the walker's path or library root; directory-entry errors without their own
path retain the containing directory. Directory-open and file-type failures
also warn. Accessible siblings still scan; symlink policy, classification,
reconciliation and retirement behavior remain unchanged.

The permission oracle proves actual PermissionDenied under a nonroot identity,
not merely a mode change that root could bypass. It verifies warnings for the
blocked subtree and a missing root, preserves an old row/progress/bytes, indexes
an accessible item and restores permissions before cleanup. Under a root test
builder, the child drops UID/GID before creating its own fixtures. Per-entry
read_dir and file_type error branches have explicit handling but were not
deterministically induced; no injection abstraction was added for that purpose.

The permission child asserts that its effective identity is nonroot and that
directory access actually fails with PermissionDenied. Its diagnostic output
reports that invariant without printing the numeric user ID. This removes the
test-only logging operation reported by CodeQL alert 16 without a suppression
or alert dismissal; the privilege drop and permission assertions remain intact.

## Replacement duration

Existing movies and episodes refresh their duration when a detectable source
change updates the fingerprint. The probe runs outside the write lock. Publication
acquires an immediate transaction before rechecking the current file, then guards
the database path and fingerprint. Controlled source replacement, a separate
connection's database fingerprint update, and replacement while publication waits
for the write lock all reject the stale probe.

A known replacement clears obsolete duration in the same transaction that clears
playback and qualifies retirement. Probe failure leaves duration unknown. An
incomplete prior fingerprint preserves its unqualified legacy value on failure;
a successful current-version probe replaces it. Unchanged files are not reprobed,
so failed probes have no automatic retry on a later unchanged scan. This change
does not refresh audio duration or other metadata and retains the documented
seconds-plus-size detection limit. The original media ID, user title/author and
progress survive the actual one-second to three-second replacement test.
