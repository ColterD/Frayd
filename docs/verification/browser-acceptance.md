# Browser acceptance

[Documentation index](../README.md) · [Development](../development.md) · [Historical review ledger](coderabbit-follow-up.md)

This records bounded manual acceptance using an isolated library and database.
It does not authorize deployment or qualify arbitrary media, browsers or public hosting.

## Upstream and native integration boundary

The working integration combines upstream v0.1.1 (`fe320ba`, through fork main `62dc`) with the fork corrections and experimental [native bundle](native-bundle.md). Current commands, measured counts and source hashes belong to the [PR description](https://github.com/ColterD/Frayd/pull/2), since concurrent integration changes can invalidate a working-tree receipt. The multi-root/settings, unavailable-root, queued-ingestion and administrator-deletion checks are HTTP/storage proof; they do not establish interactive browser acceptance. Client checks include six actual decoded-video controls alongside synthetic cases. Final commit/image qualification and actual macOS execution remain held until their gates pass; Linux results do not transfer to either Mac architecture.

## Historical review follow-up

Full review [5690203516](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5690203516)
and incremental review [5690407263](https://github.com/ColterD/Frayd/pull/2#pullrequestreview-5690407263)
are accounted for with the earlier findings in the [review ledger](coderabbit-follow-up.md).
Native follow retires external playback after transferring ownership and clears
its modal class. External follow likewise pauses native audio/video after taking
ownership, without broadcasting stale native pause events. Separate API/player readiness, current-player checks and latest
desired state prevent unready calls, stale native restarts and lost pause/seek
updates. Host/guest controls exercise fresh and retained audio/video with a mocked
YouTube API. That suite does not qualify actual external playback by itself;
the real-player checkpoint below is separate.

Party updates clamp captured timestamps under the state lock, preventing stale
updates from counting elapsed playback twice. At pre-upstream commit
`7e3a7ee6eefbbbd7c9f19223d2c9c84a19670c62`, the Docker build stage's
`cargo test --locked` passed 59 server unit tests and six archive tests (65 total)
with normal parallel execution. That historical count excludes the later upstream
and native integration. CI also requires Clippy and release compilation; warnings
in unchanged code remain. Current revision-bound commands and results are in the
[draft PR](https://github.com/ColterD/Frayd/pull/2).
That checkpoint's client/harness passes 144 Node tests and 32 actual-DOM cases, including
real native decoding, parser-gated reader restoration and real protocol setup
after an injected initial transport failure. Owned HTTP nonresponse and early
exit controls verify bounded failure and cleanup. The effective-stream regression waits
for the requested stream and the video progress request; it uses no fixed pause
delay and imposes no audio progress-save requirement. The Linux-only replacement
retirement oracle is compiled only on Linux, matching its production contract. Native progress
saves also require the latest tracker, metadata readiness and the matching stream.
The audio regression reproduces a prior local item receiving the party position;
the corrected stream keeps that old item untouched. Following party audio still
does not create a new bookmark tracker.

The harness waits for a complete port field and retries transient target-list
readiness failures within one startup deadline. Unexpected targets stay fatal.
Equals signs in owned profile paths are preserved. CI reuses an exact valid Node 22.23.4 cache entry or verifies the pinned official
archive checksum before extracting a temporary runtime, without a new action dependency; local checks require
Node 22+ with global WebSocket. Native host events retain the original media
element and check its actual state. Delayed native loads require the current
generation, playback owner, element and stream; an external handoff invalidates
them. Party Start keeps one pending request owner, and reused external players
consult the current host sender rather than a role captured by an earlier session.
Native source changes compare the actual configured stream, preventing a local
selection from confusing the last party-follow ID and another item's progress.
Server generation and socket tokens isolate replacement parties and duplicate
connections; old frames and disconnects cannot alter the current session. Host registration
also requires the creator's private proof, bound to identity and generation; a
disclosed member label alone cannot grant authority or evict its owner. A guest key
also cannot replace a registration whose outbound receiver is still open. Rejoin
works after that receiver retires; this is active-slot continuity, not first-slot
or inactive device-identity authentication. A decoded-video case proves queued stale pause suppression while preserving
a legitimate pause. Synthetic metadata fixtures explicitly model and restore
metadata readiness rather than treating an event dispatch as a state change. Complex player, party and harness contracts were
documented without changing the hosted 80% coverage threshold; its next measured
result remains a separate gate.

Current hosted outcomes, immutable source/image bindings and remaining manual
gates are reported in the [draft PR](https://github.com/ColterD/Frayd/pull/2).
Historical `1ece1df` / `5d8c4f…` results remain bound to that artifact. The historical
runtime checkpoint below uses its recorded server; no merge or live deployment
is authorized.

## Actual YouTube session checkpoint (5d63 / 767fe)

Checked 2026-10-28. Runtime source `5d63ac240e25878daa0ecb1df862c70610764b59`
builds app `sha256:767fe73fa6ae69b488588e0d5d79315cf893a3d9deb19a1b94eb94d7615e7c77`,
with server binary `ea5bebfbb197e46b6484f958c7cbd603e13cc1791f25fa424f6f15612d9f4221`
and the unchanged `70c865…` proxy. Fresh Docker Rust tests (57), Clippy, release,
full HTTP/storage integration and controlled DNS recovery pass. All qualification
projects have zero remaining containers, networks and volumes, including stopped
containers. The owned test app retains its data/config and all five sample hashes.

Two fresh isolated headless contexts load the exact production app `1d7211…`
and party `4816…` payloads. They use the actual public YouTube API and official
video `aqz-KE-bpKQ`, with both players muted. A leaves its guest session with no
sender, then hosts the same ID using its exact original player object. B leaves
its old host session and joins A; the replacement party survives.

Actual A `pauseVideo()` emits an outbound pause, B receives authoritative
`playing:false`, and both APIs report paused state 2. Actual A `playVideo()`
emits play, B receives `playing:true`, and both report state 1. B's clock advances
4.39715 → 6.90061 seconds over 2.504 seconds. This is one behavioral observation,
not a latency or general synchronization benchmark. No Party UI play/pause
shortcut or synthetic player callback supplies the result.

Both owned contexts/parties close, the browser exits, its profile is removed,
and both old room codes reject WebSocket upgrades with HTTP 404. The owner
browser is untouched. Audible output, iframe-button input, cross-machine and
long-duration playback remain unqualified; API states/clocks are not decoded
frame or physical-audio measurements.

## Reader-ready checkpoint (6810 / 5926)

Checked 2026-10-28. Tested source `6810cc2488bc1a6a0284ff2bdf140d43bd2a1fd5`
builds app `sha256:5926bed415ff0455796396218dc1851a1139818fb40588b84561666c424fa8fe`
with proxy `sha256:70c8658365d70d40979de2f191387b2e5e08bea22b44f960c0e77f751f67fb83`.
The app script is `d676ea713c0975ea5a3236ea98e1bb6e98f11b9c4ab7823d6c1165a5dee29646`.
All 31 build, 19 verification and 20 backend inputs match the tested commit.

That checkpoint passed 90 Node tests, 20 Python tests and 25 real-DOM cases,
including five native H.264 decoding controls and parser-gated reader restoration.
The normal build reuses the qualified 51-test Rust/Clippy/release stage after
confirming identical backend inputs and output binary; it is not fresh Rust
execution. New-image health and seven exact-byte HTTP asset checks pass, with
zero remaining smoke-project resources. Full local HTTP/storage and DNS suites
were **not rerun** on this image; their earlier full pass belongs to image `055`.

**The reader fix passes actual Vivaldi acceptance on image `5926`.** Three full
close/reopen cycles returned the saved raw-LF annotation and visibly painted it.
Chapter II→I restored the mark, and page 3/8 survived close/reopen. A guest
selection returned HTTP 401 with sign-in feedback and no optimistic mark.
A targeted movie check restored the actual HTTP progress value 190.989149 seconds,
then advanced to 204.628872 with readyState 4 and a decoded 1920-pixel frame.
The host and peer loaded app/party response bodies match the tested hashes;
index bytes were verified on the server, without a captured browser index body.

**That checkpoint’s party smoke was held.** Browser control detached twice at action
boundaries; the approach stopped without a reset. The earlier `1bf` image passed
the five-category and two-client party matrix. Those observations remain bound
to that image; no current two-client pass is claimed. New whitespace selection
and unsupported-selection checks were not repeated in this targeted run.
The controlled parser gate proves the lost-message lifecycle class; the precise
scheduling trigger of the earlier Vivaldi failure remains an inference.

## Hosted browser startup budget

The initial five-second discovery budget rejected a browser that later created
its owned endpoint file at **8.719 seconds** on GitHub job `123031020202`.
The diagnostic retained a failed verdict, observed only file existence and process
liveness, and never opened that late endpoint. It did not establish a policy or
DBus failure. Another matched run passed the browser cases.

The historical correction used one 15-second deadline for port discovery, the
HTTP target-list response, WebSocket opening and bootstrap commands, with a shared
60-second guard. It required loopback endpoint validation and successful protocol
setup before navigation. Bounded, redacted startup stderr remained available on
failure. The temporary late-file observer was removed after establishing the
insufficient startup budget. A controlled 8.7-second delay before launching the
same real browser distinguished the old and 15-second discovery budgets. A separate
controlled 3.2-second delay on the real target-list fetch failed under the old
three-second phase timeout and passed all 29 cases with that historical startup
deadline. These were separate controlled delays, not one combined-delay test.

The `a4027b8` harness gives startup 30 seconds for discovery, target-list response,
socket opening and bootstrap. After setup and navigation acknowledgment, the
browser case phase has a separate 60-second budget. Loopback endpoint validation,
protocol setup and bounded cleanup remain required. Local timeout controls and
49 actual-browser cases pass on this source; its hosted workflow has a separate
result gate. The 15-second values above describe only the
earlier source and hosted attempts.

## `a4027b8` actual-browser fixture checkpoint

Checked 2026-10-29 on exact source `a4027b8220b2b8c2f511ae6675d1696b2639a25b`.
The [public checkpoint](acceptance-checkpoints.json)
matches app image `sha256:670d14d9403ccfcb44a03871040ded86f0958ab1011eaadfa58c28bf151e0334`,
proxy image `sha256:8af98fa579fe88e34b3ccdc1e4e7d2cdd68ecbc57a089e7e3033f5c85eb91598`
and the served index, app script and party script bytes. The checkpoint records
PASS for 60 steps and 28 WebSocket events; the tracked
[browser harness](../../tests/browser-regression.cjs) gives the reproducible test entrypoint. A real unknown-code join
received HTTP 404. Its error-then-close sequence kept “Party not found or rejected”
visible, left Join available and left no sender. This confirms the preadmission
rejection path for that code, not every rejection cause.

The same fixture used the actual YouTube API: after guest-to-host rehosting, the
player object was retained, host pause and play reached the peer, and the peer
clock advanced 4.4386 → 6.9473 seconds. Both owned browser contexts exited,
their profiles were removed and both old room codes rejected upgrades with
HTTP 404. The five public sample copies
matched before and after the browser run. The original `/data` bind remained
opaque and unchanged. Audible output, owner Vivaldi, authenticated accounts,
other providers and live deployment remain untested here.

The separate [runtime checkpoint](acceptance-checkpoints.json)
records 49/49 actual Chrome cases on the same `a4027b8` source with 320 ms
startup and approximately 49.9 seconds in the case phase. Its local HTTP/storage
and controlled DNS suites exited 0 using the same app and proxy images; the
checkpoint binds those commands and records zero remaining resources in all three
owned QA projects. Its public CI job links preserve the hosted result boundaries.
The browser suite exercises real DOM and decoded-media controls against a
fixture; its modeled API behavior does not replace the separate actual YouTube
API and peer observation above. These a402 results do not claim that runtime
tests ran at a later documentation commit.

Earlier verification head `f16897334b55e5d6c3da201a00791c9bd4855662` had
[one complete hosted pass](https://github.com/ColterD/Frayd/actions/runs/40747689066/job/123025739721)
through browser, lint, Rust/Clippy, images and HTTP/storage/DNS integration, plus
one startup failure before any browser case. That success does not erase the
failure. The [PR checks](https://github.com/ColterD/Frayd/pull/2/checks) and current
PR evidence identify the latest tested head and hosted outcome.

The remaining sections retain historical source/image observations and their
original gates. They do not qualify later artifacts. The final
[ordering and reader readiness](#deterministic-native-ordering-and-reader-readiness)
section records the controlled product regressions. No merge or deployment is
authorized by these checks.

## Historical source and fixture boundaries

- Baseline application: `sha256:cd880c88f8c97dbebe771a3303455c714b3a03dde732cc8474634575b7f4cf58`, matching source `b852a7f87bc1ad9e3921d7b98202ccc2b719b70c`; prior accepted PR head `ccaa308e1395f0080e8c5b7de209f8ca043efb24` adds historical documentation.
- First corrected runtime source: `407afbccb94697738025ae8c394d755814e79997`, committed before its image build. Application image: `sha256:958c9f9011cbeedec1d81433a8cae89efdc58bfbd990b2e33568f31b6b656b2d`. Its pinned build passes 45 Rust tests, Clippy and release compilation; observations below apply only to this first corrected image.
- Proxy: `sha256:543d71e7db7b987f1e41dfb0a297efe5f020c141b2dbc39a859fd2473df7bbe2`.
- Separate loopback fixture, fresh guest/device and disposable account, extension-browser host and an independent in-app-browser guest for party checks. No production database or sessions were imported; only the identified free samples were copied.
- Exact copies of the five free samples were hash-checked before and after baseline testing; original live samples were not modified. The download receipts below are from 2026-10-28. Source URLs may change; these hashes identify the tested media bytes. Extract the MP4 from the movie ZIP; the comic ZIP is retained as a CBZ-compatible archive. Other samples are direct media files.

## Media provenance

| Category | Retained source and credit | Media SHA-256 |
|---|---|---|
| movie | [Sample source](https://download.blender.org/demo/movies/BBB/bbb_sunflower_1080p_30fps_normal.mp4.zip); CC BY 3.0; Blender Foundation; full embedded end credits retained. [Notice](https://peach.blender.org/about/) | `ae51005850b0ff757fe60c3dd7a12d754d3cd2397d87d939b55235e457f97658` |
| music | [Sample source](https://incompetech.com/music/royalty-free/mp3-royaltyfree/Carefree.mp3); Carefree by Kevin MacLeod (incompetech.com), Licensed under Creative Commons: By Attribution 4.0 License https://creativecommons.org/licenses/by/4.0/. [Notice](https://incompetech.com/music/royalty-free/licenses/) | `8433b770a630d9b1594fd484442c677907ece899a4d149954cd2e74fd733e311` |
| audiobook | [Sample source](https://archive.org/download/alice_in_wonderland_librivox/AliceInWonderland_librivox.m4b); Public domain in USA; LibriVox volunteer readers; complete recording. [Notice](https://librivox.org/alices-adventures-in-wonderland-by-lewis-carroll/) | `1ba22e284ee80e8cf25077bc8f10b81f0fadef6b8e61ac7964f059402e8ee1d3` |
| ebook | [Sample source](https://www.gutenberg.org/cache/epub/11/pg11-images-3.epub); Public domain in USA; Project Gutenberg notice retained. [Notice](https://www.gutenberg.org/ebooks/11) | `4f81245d31380ac1d6eafb9a321e3691369bc70985736ae4bccf849a4f784036` |
| comic | [Sample source](https://raw.githubusercontent.com/jydie5/MangaCrisp/4e851dc019cb6707444dd5a074292b6468c59f68/demo/Pepper-and-Carrot%20v01%20The-Potion-of-Flight.zip); CC BY 4.0 David Revoy; attribution packaged in archive retained. [Notice](https://www.peppercarrot.com/en/about/index.html) | `eae7b064a393cdfda43325f97c4790b92d72c339003de4467f62985c8e9fce07` |

No sample media are added to the repository by this documentation. Embedded notices/credits remain in the test copies; public-domain statements in the retained receipts are scoped to the USA.

## Earlier reviewed follow-up (858)

Source `bf3cb589881542cb0f6c4d47d467a9909c7b697d` builds application
`sha256:858879bf50c7050dfb55f93723e8fa74fbde77e4a2447e00eb574d9e45bca0b5`.
The normal build passes 51 Rust tests, Clippy and release; 43 client and 16 Python
tests also pass. This follow-up rejects selections spanning text nodes or element
boundaries, keeps raw selection text/offsets aligned, and adds conservative
interrupted-conversion recovery. The test-wrapper extraction correction is
included without a scanner suppression.

The earlier 958-image passes below remain historical observations of that exact
artifact; they are not presented as browser qualification of this later image.
At that checkpoint, browser acceptance was held for input recovery and
a fresh script-payload binding. The later owner-correlated Vivaldi tab now responds;
current-image payload binding and media rechecks remain required. At that historical checkpoint, full HTTP/storage integration and same-version stopped-volume recovery pass; these automated/storage results do not close the browser gates.

## Earlier automated qualification (9fe)

Runtime source `c4cde98a36b781ea0a0f5f512b0d5001a3fbd2fc` builds app
`sha256:9fe2f139cece83cd1151a416cb45fe8fd6e80b85bce1268cc26e002b9a276a44`
and proxy `sha256:70c8658365d70d40979de2f191387b2e5e08bea22b44f960c0e77f751f67fb83`.
Verification source `bb26bdc04d3476270ac96721af843618e8997e00` adds the browser
harness and CI wiring without changing runtime inputs. Both immutable images pass
the full HTTP/storage and controlled DNS-failure suites; their disposable projects
have zero remaining resources. All 47 client and 17 Python tests pass at that checkpoint. The normal
build reused the qualified 51-test Rust/Clippy/release stage with identical server
inputs and output binary; no fresh Rust execution is claimed for that cache hit.
Nine real-DOM cases pass separately, as described below.

At that checkpoint, the retained interactive fixture ran the earlier 858 image.
Manual media acceptance required a new image, loaded-payload binding and
actual media actions. Browser control has now recovered as described below. Earlier
958 observations, 858 backup recovery and these new automated results each retain
their own source/image limits.

## Historical acceptance matrix (baseline / 958)

| Scenario and exact action | Baseline observation | First corrected image (958) |
|---|---|---|
| Movie: open, confirm decoded playback, seek, Back, read saved progress, reopen | Decode/seek pass. Saved 203.002927 seconds; reopen starts at zero. A second seek/reopen also restarts. | Blocked on browser input recovery. A pre-binding restart observation is inconclusive; two bound-payload iterations and the actual progress response are still required. |
| Music: play, seek, close, reopen | Carefree advances; saved/resumed near 106 seconds. | Blocked on post-fix browser input recovery. |
| Audiobook: play, seek, close, reopen | Alice advances; saved/resumed near 1,076 seconds. | Blocked on post-fix browser input recovery. |
| EPUB: navigate, close, reopen | Chapter 3, page 2/3 restores. | Blocked on post-fix browser input recovery. |
| Account highlight: select real text, close, reopen | Annotation persists and text appears yellow. | Pass: HTTP 201, confirmed save message and paint, immediate chapter revisit and reopening retain the mark. |
| Guest selection | UI says Highlighted despite an unauthorized save and no persisted annotation. | Blocked on browser input recovery; automated failure/success controls pass, but a bound-payload guest selection is still required. |
| Comic: navigate, close, reopen | Page 2/4 and actual image restore. | Pass: decoded page 2/4 restores on the corrected image. |
| Distinct-client party: request, approve, queue, skip | Both clients play the requested music. | Blocked on post-fix browser input recovery. |
| Party panel Pause then Play | Guest pauses at zero while host continues beyond 47 seconds. | Blocked: both test origins time out on automated input; two-client music/video controls remain unqualified. |

## Contract and corrected paths

The correction preserves account-only highlight creation. Guest selections are
not newly made persistent: the UI must explain that sign-in is required. Account
success is reported only after the server confirms creation, updates local reading
state and paints through the existing frame protocol. Failed writes must not claim
success. Movie resume waits for source metadata. Party panel controls act on the
active player, publish its current position and preserve the server timeline.

Detailed local QA receipts retain actions, HTTP/database observations, screenshots
and source/image bindings. Automated regression tests cover the related identity
isolation correction separately; this page does not make account/device identifiers
into a private boundary for guest library/media access.

## Historical control recovery and cleanup

The owned fixture and test tabs remain available for current-source media checks.
Earlier input attempts timed out while accessibility reads worked; those receipts
remain historical control failures, not product results. The owner then opened a
uniquely tagged test URL in Vivaldi. Its exact tab appeared in the extension
provider and guest entry responded in 0.34 seconds. The provider's “Chrome” label
was not evidence of an installed Google Chrome browser. This correlation restores
control on the newly identified tab; it does not explain the earlier timeouts.

That control recovery used the older 858 image and did not qualify later images
or their loaded payloads. These test resources
remain separate from the live deployment. Browser observations do not establish
physical audio output, full-duration playback, every codec/archive, external
YouTube behavior, latency/soak/reconnect, public-hosting security or deployment.
Same-version backup/restore has its separate bounded qualification in
[Operations](../operations.md#backup-restore-and-upgrade-status).

## Automated integration follow-up

Image inputs remain those committed at `407afbccb94697738025ae8c394d755814e79997`.
Runner commit `fd67409bd97e7983e4634076843197c2f7820790` adds the existing
20-second readiness gate after the read-only-library app recreation, before
asserting exact stream bytes. This changes only the test runner; image/backend
inputs are unchanged. The earlier explicit proxy DNS-recovery deadline and proxy
identity/start-time assertions remain in place.

One initial run returned HTTP 502 immediately after that recreation. Its receipt
is retained and the cause remains unproven. One unchanged retry passed, then
the committed readiness-corrected runner passed the complete suite against the
same immutable image. Both successes do not retroactively erase the first failure
or establish its cause. This automated HTTP/storage result does not close the
blocked browser oracles above.

## Transient DNS error follow-up

Hosted head `f3206f3` had one explicit app-IP-change recovery timeout and one
successful same-head run. The failed job did not retain DNS answers, so its
specific cause remains unknown. A separate controlled regression replaced the
real app IP, verified direct health on the new app and injected one DNS SERVFAIL.
With the original configuration, the unchanged proxy retained the old peer and
returned 502 beyond 20 seconds. With `resolver_timeout 2s`, the same scenario
requeried DNS after 2.000 seconds and recovered in 2.01 seconds.

This follows the [pinned Nginx error retry path](https://github.com/nginx/nginx/blob/140aa942711868a8daeb95810bc9a81f272ada72/src/http/modules/ngx_http_upstream_zone_module.c#L761-L767):
non-NXDOMAIN errors use the resolver timeout, whose default is 30 seconds, rather
than the five-second answer-validity setting. The readiness deadline stays at
20 seconds. `tests/test_dns_recovery.py` repeats the controlled fault in CI;
the normal integration runner also captures bounded diagnostics before cleanup
if its real Docker DNS recovery gate fails. These controlled config-mounted
results do not qualify a newly built image or the held browser cases.

## Real DOM regression follow-up

The standalone `node tests/browser-regression.cjs` harness passes nine cases
against production scripts and the actual vendored player in an isolated
headless browser. It checks exact CFI/text and restored marks for plain text,
leading/trailing whitespace, existing search/highlight splits, and body style or
script text before the selection. Cross-node, next-node offset-zero and
whitespace-only selections follow the explicit no-save contract. Annotation
paint waits for HTTP 201, and a repeated saved selection sends no second POST.

The actual Plyr source setter replaces and detaches the media element twice.
Current-element play/pause/seek events reach host callbacks once per event, and
follower seeks affect that current element. Synthetic events on detached elements
can still be forwarded by the vendored library; the probe records this behavior
without claiming a naturally occurring late-event failure. Empty fixture streams
and synthetic events do not establish decoded playback or two-client timing.

The runner records the supplied browser version/executable hash and production
file hashes, and verifies cleanup of its own temporary profile/process/listener.
It uses fixture APIs and a local icon sprite override. This closes the bounded
mocked-DOM coverage gap; it does not close the held manual media matrix or
qualify an immutable browser environment. See [Development](../development.md#real-dom-regression)
for the repeatable command and browser provisioning limits.

### Browser harness cleanup correction

A hosted browser run failed while removing its temporary profile (`ENOTEMPTY`),
masking the preceding outcome. A controlled launcher/child fixture reproduced the
lifecycle gap: stopping only the launcher leaves a descendant writing and
recreating the profile. The runner now stops only its newly created process group
with bounded TERM/KILL waits before one profile removal, and reports original
runner errors separately from cleanup errors. The two lifecycle controls bring the Node total to 49; all nine
real-DOM cases also pass locally. Full Chrome was unavailable locally during that qualification. Hosted browser
checks subsequently passed on `bf40d875`; the original masked outcome remains unknown.

The controlled DNS fixture also binds only to its resolved container interface,
instead of all interfaces. Its real DNS recovery test still passes against the
same qualified images. No CodeQL suppression or dismissal was added; fresh hosted
analysis must confirm the alert disposition.

## Host echo and cleanup follow-up

A full review of `bf40d875` found that the first host party state can overwrite a
locally resumed video's source and position. The correction passes a host-only
`preserveActiveVideo` option and recognizes the exact configured stream through
the active video attribute, Plyr source child or loaded source. The guard precedes
both ID and drift branches, including stale same-media IDs. Guest initial and
subsequent synchronization retains the host's position and play/pause state.

Red Node and actual-DOM controls reproduce a host resume of 203 seconds resetting
to zero before and after metadata. The corrected source passes 67 Node tests and
18 actual-DOM cases, including fresh/stale host IDs, guest synchronization,
changed media and URL/precedence controls. Media events/transport remain synthetic
in that harness; decoded playback and distinct-client timing still need the
current-image media checks.

The DNS fixture also bounds every cleanup command to five seconds and attempts
all owned resources. It preserves an existing scenario failure when diagnostics,
removal or inventory fail, while cleanup failure remains fatal after scenario
success. Real fake-Docker hangs/errors reproduce the old masking behavior and
pass the corrected controls; all 20 Python tests pass. Three accompanying AST
command-injection diagnostics have separate static dispositions in the ledger;
cleanup timeouts are not presented as a security mitigation.

### Active-video eligibility and actual UI baseline

Candidate source `2bbedcb20fa15ee50379406706d581758c91b350`, app
`sha256:0559037b661010cb38cdddabc24de9966bed397bef529b58ca2a3706196e3f14`,
passes its full HTTP/DNS suites, but actual Vivaldi QA exposes an additional guard
case. With loaded `5e8364` app and `69fb7e` party payloads bound, a queued movie
decodes at 1920 pixels wide. After closing it, playing music, and queuing that same
movie again, the old source match leaves music playing and the movie hidden and
paused. Party state incorrectly reports the movie as playing.

The current correction captures the prior playback kind and requires an already
open video modal before preserving a matching host stream. An active paused video
remains eligible; an inactive cached video follows the reactivation path. Node
and real-DOM baseline failures precede the patch; 67 Node and 18 real-DOM cases
pass afterward. A replacement image and actual-media recheck remain required.
Earlier movie-reopen observations with stale `903` browser code are excluded from
the `055` verdict. Resource-content hashes after an explicit documented cache-bypassed
reload establish the browser payload used for the actual UI regression above.

## Native metadata resume and connected controls

Actual Vivaldi QA on `aff7ed25` received HTTP 200 with saved movie position
64.624054, yet decoded playback started near zero. This is a source-bound product
failure, separate from earlier stale-browser observations. The native regression
uses a [generated 120-second H.264 fixture](../../tests/fixtures/native-resume.mp4)
and its [generation command](../../tests/fixtures/generate-native-resume.sh).
The 24,997-byte fixture contains black 160×90 video frames, uses no downloaded
media, and was generated with the existing app image's ffmpeg 5.1.9.

The controlled native baseline observes trusted metadata from a retired media
element's cancellation clip before the replacement video has a duration. Plyr
forwards that event through its container, consuming the one-shot resume callback;
new playback settles at 0.3049 seconds despite saved progress 64. The correction
listens on the captured replacement video and checks its item, open generation and
configured stream. Closed, superseded or different-source requests cannot apply an
old resume response. Five native decoding controls now restore near 64.30–64.31
seconds, while new zero-progress playback starts near 0.315 seconds; each decodes
6–10 frames. These are controlled short-run observations, not a performance or
full-duration quality claim.

Party controls now distinguish **Hide** (remain connected) from **Leave**
(disconnect). Reopening retains the active code/state, hidden closure clears
session state, and stale socket callbacks cannot change a new connection.
Audio following retires the old video surface and pauses its player before both
fresh and retained-audio paths. The video party button uses the same reopen API.
These corrections address the actual overlay hit-test gap and the reproduced
state where video and audio remained active together.

At that earlier checkpoint, the combined source passed 85 Node tests, 20 Python
tests and 24 browser cases, including five native decoding cases. Its then-pending
image, Vivaldi and hosted gates are historical; see the current checkpoint above.

## Deterministic native ordering and reader readiness

One `eb386fe` hosted browser job restored the movie correctly at 64.3081 seconds
but failed its timing precondition: requested metadata arrived before retired
metadata. Another job passed. The corrected test explicitly holds requested bytes
until trusted retired metadata is observed, then releases them. Cache is disabled
before navigation in the harness's own fresh headless profile, with observable
held/released counts; cache bypass cannot silently turn the scenario into a pass.
A missing event fails within a bound, and both driver and runner cleanup release
holds. No existing browser profile or Vivaldi connection is controlled by this
harness protocol. The same gate reproduces the old app's native resume failure and
passes the corrected app; the fixed 150 ms timing margin is removed.

Actual `eb386fe` Vivaldi checks pass all five media categories and party controls,
but a bounded account-highlight reopen case has no yellow paint through 9.9
seconds despite a correct annotation HTTP 200. The parent `srcdoc` already contains
the prepared restore marker. One ordinary Chapter II → I round trip restores the
paint without another annotation POST. This isolates a lifecycle/delivery problem
from persistence but does not identify its exact scheduling trigger.

A real DOM parser gate reproduces the lost-listener class: the correct saved
annotation remains unpainted after the old timer sends before the reader listener
is ready. The correction waits for a generated reader-ready acknowledgement after
listeners and body load, binds restoration and messages to the active document
generation, and ignores stale book/chapter callbacks. Close clears `srcdoc` and
pending resize work. Repeated restore cannot nest duplicate marks. The opaque
origin and exact WindowProxy checks remain in force.

The combined source passes 90 Node tests, 20 Python tests and 25 browser cases,
including parser-gated restoration, actual open/close/reopen, chapter revisits,
stale ready messages and all five native H.264 controls. Earlier Vivaldi category
passes remain bound to their recorded image. The rebuilt `5926` reader passes
the three actual Vivaldi reopen cycles described above. Current two-client party
smoke remains held by browser detachment. The controlled lost-listener class is
proven; the precise trigger of the earlier live observation remains an inference.
