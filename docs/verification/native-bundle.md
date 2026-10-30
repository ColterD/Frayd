# Native bundle and macOS workflow record

[Verification records](README.md) · [Native bundle guide](../native-bundle.md) · [Development](../development.md)

The install, start, remove and check instructions are in the [native bundle guide](../native-bundle.md). This record keeps the macOS workflow decisions and the scope each native result covers.

## macOS workflow

`.github/workflows/release-mac.yml` runs on manual dispatch only, with `contents: read`. It builds and runs separate native archives on `macos-15` (ARM64) and `macos-15-intel` (x86_64), asserts the actual architecture and rejects Rosetta. Locked tests, Clippy and the two-binary build use Rust **1.99.1**. The copied compile-time source is deleted before the lifecycle suite runs. No artifact passes between jobs and no universal binary is built. The workflow has no tag trigger, release writer, service installation, owner-home install, signing, publication or deployment.

The official runner inventory on 2026-10-28, [`actions/runner-images` d53fb259895d2a1207bcdd30f288557f01288b77](https://github.com/actions/runner-images/blob/d53fb259895d2a1207bcdd30f288557f01288b77/README.md), identifies these labels and architectures (MIT; RESEARCH). Runner labels are mutable image channels, so each job records the actual image version and `uname` output. The workflow keeps the existing [`actions/checkout` 3d3c42e5aac5ba805825da76410c181273ba90b1](https://github.com/actions/checkout/blob/3d3c42e5aac5ba805825da76410c181273ba90b1/action.yml), which runs on Node 24 (MIT). It adds no setup action.

## Binary retention

The workflow uploads nothing. The upload-artifact action `cf430e030ddbb5b0abf93d22962f4752f3646cd9` (v7.0.2) failed its production-dependency advisory check on 2026-10-28, and no fixed official replacement was available. The workflow prints only nonsecret complete package manifests, archive checksums and lifecycle results into the job log, and writes a scope summary. The macOS archive is disposable runner output, not a downloadable deliverable. Before proposing retention again, check the official action's exact production dependency graph. Linux archives can be kept separately with their manifest and checksum.

## Job time limit

Each job has a 35-minute timeout, so the two jobs use at most 70 runner minutes. No measured Mac runtime is recorded here.

## Scope of the native results

`ffmpeg` and `ffprobe` are external prerequisites. The selected runner images do not establish that they exist, and the workflow does not install them. CBZ reader and basic lifecycle success does not qualify audio or video conversion, codec availability, physical playback, GPU acceleration, Intel Homebrew bottles, signed and notarized distribution, or another Linux libc or distribution. Each architecture's result stands alone.

Five real-filesystem Rust fault controls exercise install rollback. Packaged native qualification is a separate run.

## Why the helper is Rust

Compared with Bash, Rust supplies structured manifest parsing, binary-safe hashes, exclusive file creation, exact type and mode checks, and lock lifetimes tied to scope. Compared with a Python helper, it needs no separately installed interpreter at launch or removal.
