# Browser acceptance

[Documentation index](../README.md) · [Development](../development.md) · [Historical review ledger](coderabbit-follow-up.md)

This records bounded manual acceptance using an isolated library and database.
It does not authorize deployment or qualify arbitrary media, browsers or public hosting.

## Source and fixture boundaries

- Baseline application: `sha256:cd880c88f8c97dbebe771a3303455c714b3a03dde732cc8474634575b7f4cf58`, matching source `b852a7f87bc1ad9e3921d7b98202ccc2b719b70c`; prior accepted PR head `ccaa308e1395f0080e8c5b7de209f8ca043efb24` adds historical documentation.
- First corrected runtime source: `407afbccb94697738025ae8c394d755814e79997`, committed before its image build. Application image: `sha256:958c9f9011cbeedec1d81433a8cae89efdc58bfbd990b2e33568f31b6b656b2d`. Its pinned build passes 45 Rust tests, Clippy and release compilation; observations below apply only to this first corrected image.
- Proxy: `sha256:543d71e7db7b987f1e41dfb0a297efe5f020c141b2dbc39a859fd2473df7bbe2`.
- Separate loopback fixture, fresh guest/device and disposable account, Chrome host and an independent in-app-browser guest for party checks. No production database or sessions were imported; only the identified free samples were copied.
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

## Latest reviewed follow-up

Source `bf3cb589881542cb0f6c4d47d467a9909c7b697d` builds application
`sha256:858879bf50c7050dfb55f93723e8fa74fbde77e4a2447e00eb574d9e45bca0b5`.
The normal build passes 51 Rust tests, Clippy and release; 43 client and 16 Python
tests also pass. This follow-up rejects selections spanning text nodes or element
boundaries, keeps raw selection text/offsets aligned, and adds conservative
interrupted-conversion recovery. The test-wrapper extraction correction is
included without a scanner suppression.

The earlier 958-image passes below remain historical observations of that exact
artifact; they are not presented as browser qualification of this later image.
Actual latest-image browser acceptance is held for Chrome input recovery and
a fresh script-payload binding. The full latest-image HTTP/storage integration and same-version stopped-volume recovery pass; these automated/storage results do not close the browser gates.

## Acceptance matrix

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

## Limits and cleanup

The owned fixture and two test tabs remain temporarily held for manual Chrome input
recovery and the remaining oracles. Repeated mouse-input timeouts affected both
127.0.0.1 and separate-storage localhost origins while accessibility reads worked.
Input attempts stopped after the bounded failures. Cleanup is held until those
checks finish; these temporary resources are separate from the live deployment. Browser observations do not establish physical audio
output, full-duration playback, every codec/archive, external YouTube behavior,
network latency/soak/reconnect behavior, public-hosting security or deployment.
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
