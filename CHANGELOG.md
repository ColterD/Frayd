# Changelog

These notes use [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) categories.
See the [verification appendix](https://gist.github.com/ColterD/599a8a4532de0f60e31573cccb27d179) for source identities, test status and limits.
The appendix records the current source, results and remaining limits.
Historical results apply only to their named source.

## [Unreleased]

### Added

- Show build details on the sign-in screen, sidebar and Settings. Provide a command to compare a build with upstream.
- Add Recently Added shelves with date sorting and versioned local cover art.
- Add an Account entry so signed-in users can change their own password. Administrators can also use the form in Settings.
- Add an experimental native bundle for a private install directory. Check separate server and helper files before packaging.
- Give party members clear Host and Guest labels. Give the host a private proof for host controls.

### Changed

- Check library settings before saving them together. Use a separate database connection for scans.
- Use one ordered list of library roots for scans, downloads and cache checks. An explicit library override locks root edits.
- Let an empty configured root list disable the library. Keep unavailable saved roots separate from valid new roots.
- Support bounded, open-ended and suffix byte ranges for media requests. Keep converted output in a separate cache.
- Limit the files Docker can copy into an image. Limit package download waits. Let nginx resolve the app address after replacement.
- Run macOS qualification by manual request with read-only access. Check separate ARM64 and Intel archives. Publish nothing from that workflow.
- Reject optimized Python for build-context, HTTP integration and DNS qualification. Keep required assertions active before those commands create test resources.

### Removed

- Remove filename-based iTunes artwork lookup. Keep local embedded artwork.
- Remove EPUB geometry debugging and client console logging.
- Remove controls that have no action. Keep inactive design fragments as notes.

### Fixed

- Keep indexed media, progress and notes when a configured root is unavailable. Keep media IDs after source replacement.
- Hold invalid older saved root settings for repair instead of stopping startup. Show the configuration error in Settings.
- Keep saved media IDs when an older source path resolves to one unique current path. Refuse ambiguous paths or active conversion conflicts.
- Let users edit another library root while a saved root is offline. Keep strict checks for newly added roots.
- Clear only generated caches that Frayd owns. Retry interrupted conversions on later scans.
- Resolve thumbnail paths before the cleanup write transaction. Recheck database references before removing an owned thumbnail.
- Stage downloads outside library roots. Check video metadata before publication. Keep existing destination files intact.
- Preserve the selected shelf and filters when Account opens or closes. Keep the existing session policy after password changes.
- Move account deletion and permission-update database waits to blocking workers. Keep authorization and each complete transaction inside that worker.
- Report session lookup and refresh database failures as server errors. Keep invalid or expired sessions separate from database failures.
- Replace an old Saved label when the next settings save fails.
- Recheck administrator rights after the settings transaction obtains its write lock. Reject revoked rights or database errors before saving or publishing configuration.
- Show sign-in retry guidance as plain text after a rate limit. Use a five-minute fallback when the server provides no usable guidance.
- Monitor scans and downloads separately across panel changes for the same account. Report a failed library refresh even when the job finished.
- Ignore retired account, reader and playback responses. Keep saved reader positions and highlights with the current document.
- Use consistent comic and EPUB page order. Preserve supported older comic-cache order. Refresh duration and playback caches after a video source changes.
- Keep reader arrow keys usable after a reader button receives focus. Preserve normal behavior in text fields.
- Keep a typed party URL and its selection during state updates. Clear the party panel when the account changes.
- Reload local media after external party playback. Refuse party Play for a hidden or different local player and explain how to reopen it.
- Explain that saved references, conversion history or an active job can protect media from deletion. Keep its entry and source file intact.
- Stop first-owner setup when its database transaction fails. Keep account and session creation from reporting partial success.
- Check server and helper build details before creating a native archive. Report unknown build details when a metadata probe times out.
- Preserve UTF-8 library paths in Windows setup. Use an explicit child-process execution policy for the generated Windows install command.
- Report Homebrew prerequisite failures. Explain that the first login creates the owner account on a new server.
- Check native install cancellation before owned writes, between copy chunks and before publication. Roll back only paths whose recorded identities still match.
- Handle INT, TERM, HUP and QUIT during native supervision. Stop the owned server process group before removing its child records.
- Document source-install removal separately from native-bundle removal. Clarify fresh-clone upstream checks and canonical temporary-directory paths.

### Security

- Use the connection address for login limits unless trusted-proxy handling is enabled. Bound address groups and expire idle entries.
- Require the creator identity and private proof for party host registration. Keep guest device keys separate from account identities.
- Require sign-in for highlights. Check reader offsets and page fields before use.
- Refuse media deletion while saved references or active conversions protect the item. Keep original source files intact.
- Check native install-directory ownership and permissions. Refuse concurrent starts and preserve uncertain process records for inspection.

### Limits

- Download publication requires hard links. Metadata checks do not decode the complete file.
- Native cancellation cannot undo a completed publication. SIGKILL, a crash or blocked storage can leave paths that need inspection.
- Native shutdown covers descendants that stay in the owned process group. A process that leaves that group is outside this guarantee.
- Keep uncertain native child records after incomplete cleanup. This behavior does not provide complete process-tree containment.
- Public hosting, signing, physical audio and broad upgrade compatibility remain unqualified.
- An older CodeRabbit review omitted output. Support ticket #39355 tracks recovery. The omitted output remains unknown.
