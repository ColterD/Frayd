# CodeRabbit follow-up verification

Source review: [5677254865](https://github.com/ColterD/Frayd/pull/1#pullrequestreview-5677254865)
on `f2a46190c83a9d256281204a1db99edd3623983d`.

The review contains eight actionable findings in seven inline comments, two
nitpicks, and two additional diagnostics. Each item below needs a reproducible
baseline, a focused correction or evidence-backed disposition, and verification
on the final PR head. A successful review check alone does not resolve an item.

The follow-up starts at synchronized fork main
`08242a83e26a0807af5db8bd89987519b73d9494`, which includes upstream merge
`3b0d003bfb604fe49bc627d47aaf7bcefc67c11a` and preserves fork automation.
Its source tree matches the reviewed head. This draft is fork-only; deployment
requires separate owner acceptance.

| ID | Severity | Required behavior / acceptance oracle | Status |
|---|---|---|---|
| CR-01 | Major | Filter playable guest choices before limiting to 20; archives cannot hide a later song. | Pending |
| CR-02 | Major | A deliberately blocked metadata probe must not block liveness, library or progress reads; scanning eventually finishes. | Pending |
| CR-03 | Major | The proxy recovers after the app receives a different IP, without restarting the proxy. | Pending |
| CR-04 | Major | Conversion succeeds with a read-only library and writable private output cache; source bytes stay unchanged. | Pending |
| CR-05 | Minor | An incomplete OSV page cannot produce a clean audit, with or without first-page findings. | Pending |
| CR-06 | Major | Legacy ZIP filename/cache data remains readable after upgrade without losing media IDs, metadata or progress. | Pending |
| CR-07 | Major | Genuine source files with either browser-output suffix keep their IDs, content and progress. | Pending |
| CR-08 | Minor | Known artifact rows can be pruned after their files disappear; ambiguous sources and referenced rows are preserved. | Pending |
| CR-09 | Trivial | A failed Docker command exposes its captured stderr and preserves failure/cleanup behavior. | Pending |
| CR-10 | Trivial | Synthetic client environment files are excluded from build context, image and HTTP, while normal assets remain. | Pending |
| D-01 | Lint | Package/result pairing uses explicit strict semantics and mismatched counts remain fatal. | Pending |
| D-02 | Documentation | Address the reported 17.65% function documentation coverage against the 80% requirement without lowering it. | Pending |

## Safety and evidence boundaries

- Tests use disposable files, databases and Docker projects, never an operator's
  guest database or sample library.
- Generated-file ownership requires durable evidence. A filename alone cannot
  authorize deletion. Historical temporary names without ownership information
  remain ambiguous; dependent user data is never silently reassigned or removed.
- Preserve existing final transcode references. New conversions must not fall
  back to writing beside source files when their cache is unavailable.
- Preserve comic page/progress semantics and unrelated metadata. Failed archive
  refresh must not replace a usable cache with an empty result.
- Record blocked or unavailable checks explicitly. Do not substitute static
  inspection, a cached build or a unit fixture for an unobserved browser result.
