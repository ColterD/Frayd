# Experimental native bundle qualification

Native packages use a new private prefix and a separate private data directory.
Installation and removal prompt before writes; `--yes` explicitly bypasses the
prompt. Unsupported flags (including purge flags), relative/root/shared-toolchain
paths, checkout paths, overlapping bundle/prefix/data/library paths, symlink
ancestors and unknown existing prefixes/data are refused with exit 2. Decline
returns 0. Parent directories must exist. Existing prefix upgrades are unsupported.

```sh
cargo +1.99.1 build --locked --release --manifest-path server/Cargo.toml \
  --bin frayd-server --bin frayd-native
scripts/native/package.sh /absolute/new/output x86_64-unknown-linux-gnu \
  /absolute/server/target/release/frayd-server \
  /absolute/server/target/release/frayd-native
# Verify the adjacent archive checksum before extracting to a new private directory.
# Run from the extracted bundle:
./install.sh --prefix /absolute/new/prefix --data-dir /absolute/new/data
./start.sh --prefix /absolute/new/prefix --port 8484
./uninstall.sh --prefix /absolute/new/prefix
```

All runtime client assets, including vendored Plyr, accompany both binaries and
thin shell wrappers. Packaging uses the tracked client file allowlist and refuses
environment files and redirects; untracked local files are excluded. Each archive
has a SHA-256 sidecar and a manifest with base source commit, package version,
exact Rust identity, target, relative paths, types, modes, sizes and SHA-256 values.
Working-tree changes are bound by file hashes; the base commit alone does not
identify an uncommitted build. Keep the checksum and full manifest with test results.

The helper validates the complete bundle before creating anything, reserves a new
0700 prefix exclusively, stages in a private sibling directory, verifies copied
bytes and atomically renames the complete stage over its own empty reservation.
Installation creates prefix and data ownership receipts. Existing data can be
reused only with the private Frayd data marker. No existing unknown files are
adopted. Failed staging rolls back only exact newly created paths and reports
anything retained; application data is never removed. A failed removal preserves
the receipt until its payload has been removed, and missing owned payload entries
are accepted on retry using an extracted helper. Do not manually change receipt
contents to force acceptance.

Start checks unchanged payload and data ownership, holds a lifecycle lock and
supervises one foreground child. It always supplies `FRAYD_DB`, `FRAYD_THUMBS`,
`FRAYD_TRANSCODES`, `FRAYD_CLIENT_DIR`, `FRAYD_PORT` and loopback `FRAYD_BIND`.
Loopback binds never request router port mapping, regardless of `FRAYD_UPNP` or
saved UPnP settings; externally bound servers, including Docker, retain their
existing opt-in behavior.
Library roots normally come from database configuration. An explicit inherited
`FRAYD_LIBRARY` remains an intentional override and must be disjoint from prefix
and data. For an unavailable absolute library root, overlap checks resolve its
existing ancestors without creating the source directory; the server retains
configuration and saved metadata while holding source operations. Runtime data
redirects/hardlinks are refused. No source checkout is
needed after installation. The helper binary is Unix-only; non-Unix builds retain
an explicit unsupported-platform entry point so adding this binary does not force
Unix APIs into the existing Windows source bootstrap build. Windows is unqualified.

The private child receipt is an uncertainty gate. Active start/removal is refused.
Normal foreground SIGINT/SIGTERM terminates and reaps only its exact owned child
and removes the marker.
If the supervisor crashes or is killed with SIGKILL, its marker
holds subsequent start/removal even after the child exits. Inspect and resolve
that exact installation manually; PID reuse is never treated as permission to
kill a process. Keep foreground supervision alive until its server exits.
Removal verifies every remaining owned entry and rejects modified or extra files
before deleting anything. It removes only recorded unchanged payload and empty
owned directories. Data, original environment/configuration/database files,
libraries, source checkouts and shared Rust installations are never removed.
The private prefix protects against other Unix users; this is not a sandbox
against concurrent malicious code running as the same account or filesystem
administrators. Concurrent edits can hold publication or leave a partial removal;
retain the error and receipt for inspection and retry.

The Rust helper uses existing `serde`, `serde_json`, `sha2`, `hex`, `walkdir`,
`uuid` and Tokio signal/process support, with standard filesystem locking in Rust 1.99.1. No new Cargo
or runtime package is introduced. Shell wrappers contain no ownership logic.
Compared with Bash, Rust supplies structured manifest parsing, binary-safe hashes,
exclusive file creation, exact type/mode checks and lock lifetime tied to scope.
Compared with a Python runtime helper, it requires no separately installed Python
interpreter at launch/removal. Python is used only by packaging and qualification;
`/usr/bin/id` is a read-only UID probe on macOS, where `/proc/self` is unavailable.

## Reproducible checks

```sh
bash -n scripts/native/package.sh scripts/native/install.sh scripts/native/start.sh scripts/uninstall.sh
shellcheck scripts/native/package.sh scripts/native/install.sh scripts/native/start.sh scripts/uninstall.sh
CARGO_BUILD_JOBS=2 cargo +1.99.1 clippy --locked --manifest-path server/Cargo.toml \
  --bin frayd-native --target-dir target/native-qualification -- -D warnings
FRAYD_NATIVE_HELPER="$PWD/target/native-qualification/debug/frayd-native" \
python3 -m unittest discover -s tests -p test_native_bundle.py -v
```

The default suite executes real helper processes in owned temporary directories
and distinguishes its synthetic short-lived server fixture from actual HTTP server
qualification. Actual server tests require these explicit inputs:

```sh
FRAYD_NATIVE_HELPER=/absolute/qualified/frayd-native \
FRAYD_NATIVE_ARCHIVE=/absolute/package.tar.gz \
FRAYD_NATIVE_BUILD_SOURCE=/absolute/removed/compile-time/server \
python3 -m unittest discover -s tests -p test_native_bundle.py -v
```

Build the server in an owned temporary source copy and remove **that copy** before
running. Never rename/remove the active project checkout to simulate relocation.
The real runtime scenario checks installed index/vendor bytes, exact health JSON,
first-owner setup, database-selected fixture root, genuine CBZ page bytes, saved
progress after process restart, active-removal refusal, real foreground Ctrl-C
cleanup and unchanged data after removal. It uses a private synthetic account without logging credentials, tokens,
cookies, database bytes or child identifiers. It terminates only its verified
child, with a fixture-owned process group as bounded cleanup fallback.
It prints a nonsecret JSON result with architecture and package checksum.
No archive input means the actual server scenario is explicitly skipped and must
not be reported as qualified. Generic Python discovery also explicitly skips helper
scenarios without an explicitly supplied `FRAYD_NATIVE_HELPER`; it never installs/builds Rust.
`FRAYD_NATIVE_REQUIRE_QUALIFICATION=1` makes missing helper/archive/build-source
inputs a test failure in native CI. `FRAYD_NATIVE_TOOLCHAIN_FILE` optionally supplies
exported builder `rustc --version --verbose` metadata to packaging and tests; it
is recorded verbatim and compared exactly. The existing pinned Linux container
compiler is recorded as actually used, without installing/rebranding a toolchain.
Otherwise packaging queries the local exact Rust 1.99.1 toolchain. The Mac
workflow explicitly provisions 1.99.1.

## macOS workflow and held boundaries

`.github/workflows/release-mac.yml` is manual only with `contents: read`. It builds
and executes separate native archives on `macos-15` (ARM64) and `macos-15-intel`
(x86_64), asserts actual architecture and rejects Rosetta. Locked tests, Clippy and
both-bin builds use Rust **1.99.1**. The copied compile-time source is removed before
running the same actual lifecycle suite. No cross-artifact download or universal
binary is involved. Hosted execution needs coordinator authority/budget after
reviewing this concrete workflow. There is no tag trigger, release writer, service
installation, owner-home install, release signing, publication or deployment.

As checked 2026-10-28, official runner inventory at
[`actions/runner-images` d53fb259895d2a1207bcdd30f288557f01288b77](https://github.com/actions/runner-images/blob/d53fb259895d2a1207bcdd30f288557f01288b77/README.md)
identifies these labels and architectures (MIT; RESEARCH). Runner labels are mutable
image channels; the job records actual image/version and uname output. Existing
[`actions/checkout` 3d3c42e5aac5ba805825da76410c181273ba90b1](https://github.com/actions/checkout/blob/3d3c42e5aac5ba805825da76410c181273ba90b1/action.yml)
uses Node 24 (MIT; retained existing action). No extra setup action is added.

**Binary retention is held.** The coordinator rejected upload-artifact
`cf430e030ddbb5b0abf93d22962f4752f3646cd9` (v7.0.2) after its production-dependency
advisory check; no qualified fixed official replacement was available in that
receipt. The workflow introduces no upload/download/release dependency. It prints
only nonsecret complete package manifests, archive checksums and lifecycle results
in retained job logs and writes a scope summary. The macOS archive is disposable
runner output, **not** a downloadable deliverable. Recheck the official action's
exact production dependency graph before proposing retention. Local Linux archives
can be retained separately with their manifest and checksum.

Each job has a 35-minute timeout: two jobs permit at most 70 runner-minutes.
At the coordinator-reported published rate of $0.062/minute, that arithmetic cap is
$4.34; an **unmeasured forecast** of 8–20 minutes/job gives 16–40 runner-minutes
($0.992–$2.48). Neither estimate establishes account billing, remaining allowance,
queue time or measured Mac runtime. Coordinator must verify the account-specific
budget before dispatch; no hosted run has been authorized by this document.

`ffmpeg` and `ffprobe` are external media prerequisites. The selected runner images
do not establish their availability, and this workflow does not install them.
CBZ reader/basic lifecycle success does not qualify audio/video conversion,
codec availability, physical playback, GPU acceleration, Intel Homebrew bottles,
signed/notarized distribution, another Linux libc/distro, or actual macOS execution
until those platform jobs have passed. Keep each architecture's result separate.
