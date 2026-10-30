# Native bundle

[Documentation index](README.md) · [Quick start](../README.md) · [Operations](operations.md) · [Development](development.md)

The native bundle runs the Frayd server without Docker, from a private install prefix and a separate private data directory. It is experimental. The [Docker quick start](../README.md#run-with-docker) is the reproducible way to start Frayd.

Install and uninstall prompt before they write anything. `--yes` skips the prompt.

## Build and package

```sh
cargo +1.99.1 build --locked --release --manifest-path server/Cargo.toml \
  --bin frayd-server --bin frayd-native
python3 scripts/native/package.py /absolute/new/output x86_64-unknown-linux-gnu \
  /absolute/server/target/release/frayd-server \
  /absolute/server/target/release/frayd-native
```

The target is `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin` or `x86_64-apple-darwin`. Both binaries must be absolute regular files built for that target, and the output directory must be new. macOS archives are unsigned. `ffmpeg` and `ffprobe` are not bundled.

The archive holds both binaries, thin shell wrappers and every runtime client asset, including vendored Plyr. Packaging takes client files from the tracked allowlist, so untracked local files stay out, and it refuses environment files and redirects. Each archive has a SHA-256 sidecar. `manifest.json` records the base source commit, package version, exact Rust identity, target, and every file's relative path, type, mode, size and SHA-256. `build-info.json` is copied from the server binary and records its identity. Before creating output, packaging refuses identical server/helper bytes and checks the helper role and both binaries' version, commit, dirty state and tag distance. `frayd-native --build-info` reads compile-time metadata and exits without installing, starting or removing anything. The helper reports `role: "frayd-native"`. This detects accidental binary substitution; it does not authenticate a binary supplied by a caller. A base commit does not identify an uncommitted build, but the file hashes do. Keep the checksum and manifest with your test results.

## Install, start and remove

Verify the archive checksum, then extract into a new private directory. Extraction must preserve the manifest modes, including under umask 077:

```sh
mkdir -m 700 /absolute/new/extraction-directory
tar -xpzf /absolute/package.tar.gz -C /absolute/new/extraction-directory
```

Run these from the extracted bundle. The parent directories of the prefix and data directory must exist:

```sh
./install.sh --prefix /absolute/new/prefix --data-dir /absolute/new/data
./start.sh --prefix /absolute/new/prefix --port 8484
./uninstall.sh --prefix /absolute/new/prefix
```

### Install

Install exits with code 2 and changes nothing when it finds:

- an unsupported flag, including any purge flag
- a relative path, the filesystem root, or a shared toolchain path
- a checkout path, or overlapping bundle, prefix, data and library paths
- an ancestor that is a symlink or not a directory, is owned by neither root nor the invoking user, or is writable by group or others without the sticky bit (a canonical sticky temporary directory stays allowed; on macOS `/tmp` is a symlink, so use `/private/tmp` or resolve the parent with `pwd -P`)
- an existing prefix, or existing data that Frayd did not create

Start and removal repeat the ancestor checks. Declining the prompt returns 0. Upgrading an existing prefix is not supported.

The helper checks the whole bundle before it creates anything. It reserves a new prefix with mode 0700, stages the files in a private sibling directory, verifies the copied bytes, and renames the staged tree over its own empty reservation. It writes ownership receipts for the prefix and the data directory. Existing data is reused only when its private Frayd data marker matches its canonical path and ownership checks; a retained marked data directory can be reused on a later install with a new prefix. No other existing file is adopted.

Installation errors and handled INT/TERM/HUP/QUIT cancellation return code 2. If initial prefix metadata cannot be recorded, the unqualified prefix is retained and reported with the original error. If initial stage metadata cannot be recorded, the helper preserves that unqualified stage, reports its exact path and the original error, and removes only its still-matching empty prefix reservation. Inspect any retained stage before manual recovery; its presence grants no cleanup authority. If an install returns an ordinary error, the helper removes only the exact paths it created and reports anything it kept. After consent and before its first owned write, installation blocks INT, TERM, HUP and QUIT on its single thread. Pending cancellation is checked between 1 MiB copy chunks and before atomic publication; cancellation rolls back the attempt-owned paths while the signals remain blocked. Changed or uncertain path identities and unknown staging entries are retained and reported. Once publication commits, cancellation does not undo the installed tree. Pending cancellation signals are consumed after cleanup or commit, then the original signal mask is restored. SIGKILL, a crash or an uninterruptible filesystem operation can still leave a reservation or private `.frayd-stage-*` sibling; retain and inspect those paths before manual recovery. It never removes application data. It reports newly created data directories that are marked or uncertain. A metadata failure during rollback does not replace the original error or skip later cleanup.

### Start

Start first refuses an ignored, nondefault or `SA_NOCLDWAIT` SIGCHLD disposition before creating child receipts or spawning a server; it preserves the inherited disposition. It verifies the installed files and the data ownership, takes a lifecycle lock and supervises one foreground server in its own process group. It always sets `FRAYD_DB`, `FRAYD_THUMBS`, `FRAYD_TRANSCODES`, `FRAYD_CLIENT_DIR`, `FRAYD_PORT` and a loopback `FRAYD_BIND`. A loopback bind never requests router port mapping, whatever `FRAYD_UPNP` or the saved UPnP setting says. Externally bound servers, including Docker, keep their opt-in behavior.

The server receives EOF on standard input and inherits the supervisor’s stdout and stderr. Interactive server input is unsupported. Start refuses before child receipts or spawning if either output terminal has `TOSTOP` enabled; disable that flag before starting or redirect both output streams to files, pipes or `/dev/null`. The helper never changes terminal flags or ignores terminal job-control signals. Ctrl-Z suspends only the foreground supervisor while the server group continues running; resume the supervisor before stopping it normally.

Library roots normally come from the database configuration. An inherited `FRAYD_LIBRARY` still overrides them and must not overlap the prefix or data directory. When an absolute library root is unavailable, overlap checks resolve its existing ancestors without creating the directory. The server keeps its configuration and saved metadata and holds source operations until the root returns. Redirects and hardlinks in the runtime data are refused. No source checkout is needed after installation.

While the server runs, a private receipt marks it as owned, and start and removal refuse to run against an active one. Ctrl-C, SIGTERM, SIGHUP or SIGQUIT in the foreground sends TERM to the owned server group, allows a short grace period, then sends KILL while its leader is still unreaped, checking `WNOWAIT` child custody immediately before each termination signal. It reaps the leader and removes both child receipts only after the group disappears. If the supervisor crashes or is killed with SIGKILL, the receipt stays and blocks start and removal even after the server exits. Inspect that installation by hand. The helper never kills a process because its PID was reused. Keep the foreground supervisor alive until its server exits. Downloader and transcoder descendants that remain in the owned process group are included. An observed live child in a changed group, a surviving or inaccessible group, or a shutdown timeout retains both receipts for inspection. Some Unix systems hide an exited child from group lookup before it is reaped; a missing lookup is accepted only after a fresh wait confirms that owned child exited and remains unreaped. A missing group signal is only cleanup progress: receipt removal still requires reaping and a subsequent signal-zero disappearance check. No termination signal is sent using a group ID after its leader is reaped; a reused ID causes a conservative hold. Descendants that deliberately leave the group with `setsid` or `setpgid` are outside this containment guarantee, so this is not whole-system process-tree tracking.

The running server also records itself in `.frayd-child.json` in the data directory, in the same pid-and-state format as the prefix receipt. If two prefixes share one data directory, the second refuses to start while the first server runs and after a supervisor crash. A crash leaves both receipts. Before you recover by hand, verify that the old server has stopped. The helper never kills a process by PID and never adopts an existing one.

### Remove

Uninstall verifies every remaining owned entry and rejects modified or extra files before it deletes anything. It removes only the recorded unchanged payload and the empty directories it owns. It never removes data, environment files, configuration, databases, libraries, source checkouts or shared Rust installations.

Any entry the receipt does not list is unowned and stops removal. On macOS, Finder can add a `.DS_Store` file to the prefix when you browse it, and that file blocks removal with an `unowned entry: .DS_Store` error. Delete only that file yourself, then run uninstall again.

Removal does not need the data directory. If the data is missing or moved, or its marker is missing or malformed, the prefix receipt and the unchanged payload still authorize removal. A recorded data path that is redirected or overlaps still blocks it. Start keeps requiring a valid data marker. Removal never recreates or reads the data location.

A failed removal keeps the receipt until the payload is gone. A retry accepts owned entries that are already missing, and runs from an extracted helper, so keep the extracted bundle until removal succeeds. Do not edit receipts by hand to force acceptance.

### Limits

The private prefix and the ancestor checks keep other Unix users out. The checks read owner and mode on each ancestor path. They do not walk the path atomically relative to a directory handle. A race by the same user or by root, and access granted through ACLs, are outside the guarantee. The bundle is not a sandbox against malicious code running as the same account, or against filesystem administrators. Concurrent edits can block publication or leave a partial removal. Keep the error and the receipt, inspect, and retry.

The helper binary is Unix-only. Non-Unix builds keep an explicit unsupported-platform entry point, so the Windows source bootstrap build still compiles. Windows is not supported.

`frayd-native` holds all ownership logic and the shell wrappers only call it. It uses crates the server already depends on (`serde`, `serde_json`, `sha2`, `hex`, `walkdir`, `uuid` and Tokio), plus a direct `libc` dependency for `geteuid` at the version Tokio already locks, and Rust 1.99.1 standard filesystem locking. It needs no Python at launch or removal. It reads the effective user ID with `geteuid`, so Linux and macOS use the same path.

## Checks

Run from the repository root. The Rust commands use a separate target directory:

```sh
bash -n scripts/native/install.sh
bash -n scripts/native/start.sh
bash -n scripts/uninstall.sh
shellcheck scripts/native/install.sh scripts/native/start.sh scripts/uninstall.sh
CARGO_BUILD_JOBS=2 cargo +1.99.1 clippy --locked --manifest-path server/Cargo.toml \
  --bin frayd-native --target-dir target/native-qualification -- -D warnings
CARGO_BUILD_JOBS=2 cargo +1.99.1 build --locked --manifest-path server/Cargo.toml \
  --bin frayd-native --bin frayd-server --target-dir target/native-qualification
FRAYD_NATIVE_HELPER="$PWD/target/native-qualification/debug/frayd-native" \
python3 -m unittest discover -s tests -p test_native_bundle.py -v
```

The package tests use the `frayd-server` binary beside the helper, so the build step builds both binaries. The default suite runs the real helper in owned temporary directories against a short-lived synthetic server. Tests that need the real server require these inputs:

```sh
FRAYD_NATIVE_HELPER=/absolute/qualified/frayd-native \
FRAYD_NATIVE_ARCHIVE=/absolute/package.tar.gz \
FRAYD_NATIVE_BUILD_SOURCE=/absolute/removed/compile-time/server \
python3 -m unittest discover -s tests -p test_native_bundle.py -v
```

Build the server in a temporary copy of the source and delete that copy before running the suite. Do not rename or remove your checkout to simulate relocation.

The real-server scenario checks:

- the installed index and vendored asset bytes, and the exact health JSON
- first-owner setup with a private synthetic account, without logging credentials, tokens, cookies, database bytes or child identifiers
- a database-selected fixture library root, and genuine CBZ page bytes
- saved progress after a process restart
- refusal to remove while the server is active
- real foreground Ctrl-C cleanup, and unchanged data after removal

The scenario stops only the server it started, and falls back to the fixture's own process group for cleanup.

Darwin exited-child lookup handling has a simulated branch-contract check on Linux; that is not a macOS runtime pass. Actual macOS lifecycle qualification remains required.

Without an archive input, the real-server scenario is skipped, and a skip is not a pass. Generic Python discovery also skips the helper scenarios unless `FRAYD_NATIVE_HELPER` is set, and it never installs or builds Rust. Set `FRAYD_NATIVE_REQUIRE_QUALIFICATION=1` to make a missing helper, archive or build-source input a failure. Native CI sets it.

`FRAYD_NATIVE_TOOLCHAIN_FILE` can supply exported `rustc --version --verbose` metadata from the builder. Packaging records it verbatim and the tests compare it exactly. The pinned Linux container compiler is recorded as it is, without installing or renaming a toolchain. Without that variable, packaging queries the local Rust 1.99.1 toolchain.
