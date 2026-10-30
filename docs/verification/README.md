# Verification records

These records belong to [PR #2](https://github.com/ColterD/Frayd/pull/2) on the `ColterD/Frayd` fork. The upstream project does not carry this directory. They hold what the guides leave out: review findings, test counts, commit and image identities, and acceptance notes.

| File | Contents |
|---|---|
| [pr-2-changelog.md](pr-2-changelog.md) | What the PR changes by area, with a row for every changed file and every review finding |
| [coderabbit-follow-up.md](coderabbit-follow-up.md) | The review ledger: each finding, its fix and the check that shows it |
| [browser-acceptance.md](browser-acceptance.md) | Manual and real-browser acceptance checkpoints, with the images and media used |
| [native-bundle.md](native-bundle.md) | The macOS workflow decisions and the scope of the native results |
| [acceptance-checkpoints.json](acceptance-checkpoints.json) | Per-commit QA results, artifact digests and hosted job links |

Each result applies to the source, image and scenario named beside it. A later head needs a new run. The PR page shows the hosted checks for the current head. To run the checks yourself, use [Development](../development.md#canonical-checks).
