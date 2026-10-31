//! Native lifecycle authority: no guessed ownership, recursive deletion, or service control.
//! Path checks exclude untrusted Unix ancestors; they do not reserve names against
//! concurrent changes by the invoking user or root, and do not inspect ACL grants.
// Share the server's compile-time identity without changing its API. The helper
// does not expose the HTTP handler from this module.
#[cfg(unix)]
#[path = "../build_info.rs"]
#[allow(dead_code)]
mod build_info;

#[cfg(unix)]
mod native {
    use serde::{Deserialize, Serialize};
    use sha2::{Digest, Sha256};
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs::{self, DirBuilder, File, OpenOptions};
    use std::io::{self, Read, Seek, Write};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::path::{Component, Path, PathBuf};
    use std::process::ExitCode;
    use walkdir::WalkDir;

    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
    const RECEIPT: &str = ".frayd-install.json";
    const DATA_RECEIPT: &str = ".frayd-data.json";
    const LOCK: &str = ".frayd-lock";
    const CHILD: &str = ".frayd-child.json";

    const INSTALL_SIGNALS: [i32; 4] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT];

    /// Install is single-threaded. Block cancellation before creating owned paths,
    /// inspect pending signals at bounded checkpoints, and keep the mask through rollback.
    struct InstallSignals {
        previous: libc::sigset_t,
    }
    impl InstallSignals {
        fn block() -> Result<Self> {
            // SAFETY: initialized sets and valid pointers are passed to POSIX APIs.
            unsafe {
                let mut blocked = std::mem::zeroed();
                let mut previous = std::mem::zeroed();
                libc::sigemptyset(&mut blocked);
                for signal in INSTALL_SIGNALS {
                    libc::sigaddset(&mut blocked, signal);
                }
                let error = libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, &mut previous);
                if error != 0 {
                    return Err(io::Error::from_raw_os_error(error).into());
                }
                Ok(Self { previous })
            }
        }
        fn checkpoint(&self) -> Result<()> {
            // SAFETY: sigpending initializes the valid set passed to sigismember.
            unsafe {
                let mut pending = std::mem::zeroed();
                if libc::sigpending(&mut pending) != 0 {
                    return Err(io::Error::last_os_error().into());
                }
                if INSTALL_SIGNALS
                    .iter()
                    .any(|signal| libc::sigismember(&pending, *signal) == 1)
                {
                    return reject("installation cancelled; rolling back attempt-owned paths");
                }
            }
            Ok(())
        }
    }
    impl Drop for InstallSignals {
        fn drop(&mut self) {
            // Cleanup/commit has finished. Consume each currently pending blocked
            // cancellation with sigwait, then restore the caller's original mask.
            // A later signal follows the original disposition; no rollback follows commit.
            // SAFETY: only this installation thread consumes these blocked signals.
            unsafe {
                for signal in INSTALL_SIGNALS {
                    let mut pending = std::mem::zeroed();
                    if libc::sigpending(&mut pending) == 0
                        && libc::sigismember(&pending, signal) == 1
                    {
                        let mut one = std::mem::zeroed();
                        libc::sigemptyset(&mut one);
                        libc::sigaddset(&mut one, signal);
                        let mut consumed = 0;
                        libc::sigwait(&one, &mut consumed);
                    }
                }
                let error =
                    libc::pthread_sigmask(libc::SIG_SETMASK, &self.previous, std::ptr::null_mut());
                if error != 0 {
                    eprintln!(
                        "could not restore installation signal mask: {}",
                        io::Error::from_raw_os_error(error)
                    );
                }
            }
        }
    }
    /// The path must still name this attempt's inode before rollback may remove it.
    struct CreatedPath {
        path: PathBuf,
        identity: fs::Metadata,
    }
    impl CreatedPath {
        fn record(path: PathBuf) -> Result<Self> {
            let identity = fs::symlink_metadata(&path)?;
            Ok(Self { path, identity })
        }
        fn remove(&self) -> Result<()> {
            let current = fs::symlink_metadata(&self.path)?;
            if current.dev() != self.identity.dev()
                || current.ino() != self.identity.ino()
                || current.file_type() != self.identity.file_type()
            {
                return reject("attempt-owned path changed; retained for inspection");
            }
            if current.is_dir() {
                fs::remove_dir(&self.path)?;
            } else {
                fs::remove_file(&self.path)?;
            }
            Ok(())
        }
    }
    fn copy_install_file(
        source: &mut File,
        destination: &mut File,
        signals: &InstallSignals,
    ) -> Result<()> {
        let mut buffer = vec![0; 1024 * 1024];
        loop {
            signals.checkpoint()?;
            let count = source.read(&mut buffer)?;
            if count == 0 {
                return Ok(());
            }
            signals.checkpoint()?;
            destination.write_all(&buffer[..count])?;
        }
    }
    fn publish_install(stage: &Path, prefix: &Path, signals: &InstallSignals) -> Result<()> {
        signals.checkpoint()?;
        fs::rename(stage, prefix)?;
        Ok(())
    }

    /// Payload entry type, serialized as "file" or "dir".
    #[derive(Deserialize, Serialize, PartialEq, Eq)]
    #[serde(rename_all = "lowercase")]
    enum Kind {
        File,
        Dir,
    }
    /// Serialized fingerprint of one relative payload path; unknown fields are refused.
    #[derive(Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct Entry {
        path: String,
        kind: Kind,
        mode: u32,
        size: u64,
        sha256: String,
    }
    /// Untrusted package identity and payload inventory, validated before creation.
    #[derive(Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct Manifest {
        schema: u32,
        version: String,
        source: String,
        target: String,
        toolchain: String,
        entries: Vec<Entry>,
    }
    /// Prefix-bound ownership record; embedded fingerprints authorize exact payload only.
    #[derive(Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct Receipt {
        schema: u32,
        prefix: PathBuf,
        data: PathBuf,
        data_id: String,
        manifest: Manifest,
        manifest_file: Entry,
    }
    /// Private data reuse identity that deliberately grants no data deletion authority.
    #[derive(Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct DataReceipt {
        schema: u32,
        app: String,
        path: PathBuf,
        id: String,
    }
    /// Refuse with a nonsecret diagnostic; the CLI exits 2.
    fn reject<T>(message: &str) -> Result<T> {
        Err(message.into())
    }
    /// Create a new 0700 directory; an existing path is an error and is never adopted.
    fn private_dir(path: &Path) -> Result<()> {
        DirBuilder::new().mode(0o700).create(path)?;
        Ok(())
    }
    /// Create a new private metadata file from caller-serialized receipt bytes.
    /// Existing paths are never replaced. A write/sync failure removes only the
    /// same newly opened inode when possible, otherwise reports retained metadata.
    fn private_file(path: &Path, bytes: &[u8]) -> Result<File> {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
            let opened = file.metadata()?;
            let current = fs::symlink_metadata(path)?;
            if opened.dev() == current.dev() && opened.ino() == current.ino() {
                fs::remove_file(path).map_err(|cleanup| {
                    format!(
                        "write failed: {error}; partial private file retained at {}: {cleanup}",
                        path.display()
                    )
                })?;
            }
            return Err(error.into());
        }
        Ok(file)
    }
    /// Remove metadata only while the path still names the inode this attempt created.
    fn remove_private_file(path: &Path, file: &File) -> Result<()> {
        let opened = file.metadata()?;
        let current = fs::symlink_metadata(path)?;
        if current.dev() != opened.dev() || current.ino() != opened.ino() {
            return reject("private metadata changed; retained for inspection");
        }
        fs::remove_file(path)?;
        Ok(())
    }
    /// Update the originally created child marker without reopening its pathname.
    fn record_child(file: &mut File, pid: Option<u32>) -> Result<()> {
        file.rewind()?;
        file.set_len(0)?;
        file.write_all(&serde_json::to_vec(
            &serde_json::json!({"pid": pid, "state": "running"}),
        )?)?;
        file.sync_all()?;
        Ok(())
    }
    /// Compare containment in both directions for already validated absolute paths.
    /// This lexical predicate performs no I/O and does not resolve symlink aliases;
    /// callers must establish canonical spelling and safe ancestors separately.
    fn disjoint(a: &Path, b: &Path) -> bool {
        !a.starts_with(b) && !b.starts_with(a)
    }
    /// Resolve a configured media path without creating missing source directories.
    /// Existing paths keep canonical alias resolution. For an absent absolute root,
    /// resolve its nearest existing ancestor and append only normal components;
    /// inaccessible, dangling or otherwise unresolvable ancestors fail closed.
    fn prospective_library_path(path: &Path) -> Result<PathBuf> {
        match fs::canonicalize(path) {
            Ok(resolved) => return Ok(resolved),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        if !path.is_absolute()
            || path
                .components()
                .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
        {
            return reject("missing library must be an absolute path with normal components");
        }
        let mut ancestor = path;
        let mut suffix = Vec::new();
        loop {
            match fs::symlink_metadata(ancestor) {
                Ok(_) => {
                    let mut resolved = fs::canonicalize(ancestor)?;
                    if !resolved.is_dir() {
                        return reject("library ancestor is not a directory");
                    }
                    for component in suffix.iter().rev() {
                        resolved.push(component);
                    }
                    return Ok(resolved);
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    suffix.push(
                        ancestor
                            .file_name()
                            .ok_or("unresolvable library ancestor")?
                            .to_os_string(),
                    );
                    ancestor = ancestor.parent().ok_or("unresolvable library ancestor")?;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    /// Preflight an absolute target against protected paths and unsafe ancestors.
    /// Reject ambiguous spelling, symlinks, checkouts, shared toolchains and overlap
    /// with an explicit library before mutation; this does not reserve the path.
    fn safe_path(path: &Path) -> Result<()> {
        let spelling = path.to_str().ok_or("non UTF-8 target refused")?;
        if spelling
            .split('/')
            .skip(1)
            .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return reject("target spelling must be canonical without empty/dot components");
        }
        if !path.is_absolute()
            || path == Path::new("/")
            || path
                .components()
                .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
        {
            return reject("target must be an absolute, non-root path without dot components");
        }
        for blocked in [
            "/usr",
            "/bin",
            "/sbin",
            "/lib",
            "/lib64",
            "/etc",
            "/System",
            "/Library",
            "/Applications",
            "/opt",
            "/boot",
            "/dev",
            "/proc",
            "/sys",
            "/run",
        ] {
            if path.starts_with(blocked) {
                return reject("shared system target refused");
            }
        }
        for part in path.components() {
            if let Component::Normal(name) = part {
                if [".cargo", ".rustup", ".git", ".config", ".ssh"]
                    .iter()
                    .any(|v| name == *v)
                {
                    return reject("shared configuration/toolchain target refused");
                }
            }
        }
        for ancestor in path.ancestors() {
            match fs::symlink_metadata(ancestor) {
                Ok(meta) => {
                    if meta.file_type().is_symlink() || !meta.is_dir() {
                        return reject("symlink or non-directory target ancestor refused");
                    }
                    // SAFETY: geteuid has no preconditions and cannot fail.
                    let current_uid = unsafe { libc::geteuid() };
                    if meta.uid() != 0 && meta.uid() != current_uid {
                        return reject("target ancestor owned by another user refused");
                    }
                    if meta.mode() & 0o022 != 0 && meta.mode() & 0o1000 == 0 {
                        return reject("group/other-writable non-sticky target ancestor refused");
                    }
                    if ancestor.join(".git").exists() {
                        return reject("source checkout target refused");
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
        if let Some(library) = std::env::var_os("FRAYD_LIBRARY") {
            if !library.is_empty()
                && !disjoint(path, &prospective_library_path(Path::new(&library))?)
            {
                return reject("library overlap refused");
            }
        }
        Ok(())
    }
    /// Require an existing 0700 directory owned by the effective Unix user.
    /// Passing this check alone does not establish payload ownership.
    fn private_owned_dir(path: &Path) -> Result<()> {
        let meta = fs::symlink_metadata(path)?;
        // SAFETY: geteuid has no preconditions and cannot fail.
        let current_uid = unsafe { libc::geteuid() };
        if !meta.is_dir()
            || meta.file_type().is_symlink()
            || meta.mode() & 0o777 != 0o700
            || meta.uid() != current_uid
        {
            return reject("directory must be private (0700) and owned by the invoking user");
        }
        Ok(())
    }
    /// Describe one regular file or directory for manifest comparison without writes.
    /// Reject redirects, hardlinks, special files and special modes; hash regular
    /// file bytes. The caller remains responsible for validating the relative name.
    fn entry(path: &Path, relative: &str) -> Result<Entry> {
        let meta = fs::symlink_metadata(path)?;
        if meta.file_type().is_symlink()
            || (!meta.is_file() && !meta.is_dir())
            || meta.mode() & 0o7000 != 0
            || (meta.is_file() && meta.nlink() != 1)
        {
            return reject("symlink, hardlink, special file or special mode refused");
        }
        let hash = if meta.is_file() {
            let mut file = File::open(path)?;
            let mut digest = Sha256::new();
            let mut buffer = [0; 65536];
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                digest.update(&buffer[..n]);
            }
            hex::encode(digest.finalize())
        } else {
            String::new()
        };
        Ok(Entry {
            path: relative.to_owned(),
            kind: if meta.is_file() {
                Kind::File
            } else {
                Kind::Dir
            },
            mode: meta.mode() & 0o777,
            size: if meta.is_file() { meta.len() } else { 0 },
            sha256: hash,
        })
    }
    /// Validate the complete manifest identity, path namespace and payload contract.
    /// Require unique safe relative names and correctly typed executable entrypoints;
    /// return the allowed names without filesystem effects. This is not a signature.
    fn entry_names(manifest: &Manifest) -> Result<BTreeSet<String>> {
        if manifest.schema != 1
            || manifest.version.is_empty()
            || manifest.target.is_empty()
            || manifest.toolchain.is_empty()
            || manifest.source.len() != 40
            || !manifest.source.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return reject("invalid manifest identity");
        }
        let mut names = BTreeSet::new();
        for item in &manifest.entries {
            let path = Path::new(&item.path);
            let top = item.path.split('/').next().unwrap_or("");
            if ![
                "frayd-server",
                "frayd-native",
                "client",
                "install.sh",
                "start.sh",
                "uninstall.sh",
                "README.md",
                "build-info.json",
            ]
            .contains(&top)
                || (top != "client" && item.path.contains('/'))
            {
                return reject(
                    "unexpected bundle payload; configuration and data are not install-owned",
                );
            }
            if path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
                || item.path.contains('\\')
                || path.components().count() != item.path.split('/').count()
                || item.mode > 0o777
                || !names.insert(item.path.clone())
            {
                return reject("invalid, duplicate or redirecting manifest path/type/mode");
            }
            if item.kind == Kind::Dir
                && (item.size != 0 || !item.sha256.is_empty() || item.mode & 0o700 != 0o700)
            {
                return reject("invalid directory entry");
            }
            if item.kind == Kind::File
                && (item.sha256.len() != 64 || !item.sha256.bytes().all(|c| c.is_ascii_hexdigit()))
            {
                return reject("invalid checksum");
            }
        }
        for (required, kind, executable) in [
            ("frayd-server", Kind::File, true),
            ("frayd-native", Kind::File, true),
            ("install.sh", Kind::File, true),
            ("start.sh", Kind::File, true),
            ("uninstall.sh", Kind::File, true),
            ("README.md", Kind::File, false),
            ("client", Kind::Dir, false),
            ("client/index.html", Kind::File, false),
        ] {
            let Some(item) = manifest.entries.iter().find(|item| item.path == required) else {
                return reject("incomplete bundle");
            };
            if item.kind != kind || (executable && item.mode & 0o100 == 0) {
                return reject("invalid required payload type or executable mode");
            }
        }
        Ok(names)
    }
    /// Compare an owned candidate with its recorded type, mode, size and checksum.
    /// Missing entries are permitted only when the caller enables interrupted-removal
    /// recovery; changed existing entries always fail before deletion authority.
    fn verify_entry(base: &Path, expected: &Entry, allow_missing: bool) -> Result<()> {
        let path = base.join(&expected.path);
        if allow_missing && !path.try_exists()? {
            return Ok(());
        }
        let actual = entry(&path, &expected.path)?;
        if actual.kind != expected.kind
            || actual.mode != expected.mode
            || actual.size != expected.size
            || actual.sha256 != expected.sha256
        {
            return Err(format!("modified bundle/install entry: {}", expected.path).into());
        }
        Ok(())
    }
    /// Verify recorded payload and reject every unlisted or redirected tree entry.
    /// The caller supplies narrowly allowed metadata names and validates their own
    /// contents separately. Missing payload is accepted only for removal retry.
    fn verify_tree(
        base: &Path,
        manifest: &Manifest,
        extra: &[&str],
        allow_missing: bool,
    ) -> Result<()> {
        let mut names = entry_names(manifest)?;
        for item in &manifest.entries {
            verify_entry(base, item, allow_missing)?;
        }
        for name in extra {
            names.insert((*name).to_owned());
        }
        for found in WalkDir::new(base).follow_links(false).min_depth(1) {
            let found = found?;
            let relative = found
                .path()
                .strip_prefix(base)?
                .to_str()
                .ok_or("non UTF-8 path")?;
            if !names.contains(relative) {
                return Err(format!("unowned entry: {relative}").into());
            }
            if found.file_type().is_symlink() {
                return reject("symlink in bundle/install");
            }
        }
        Ok(())
    }
    /// Read and validate an extracted bundle before any installation writes.
    /// Return its parsed payload contract plus the manifest file fingerprint, which
    /// is recorded separately because the manifest cannot checksum itself.
    fn load_bundle(base: &Path) -> Result<(Manifest, Entry)> {
        let manifest_file = entry(&base.join("manifest.json"), "manifest.json")?;
        if manifest_file.kind != Kind::File {
            return reject("manifest is not a regular file");
        }
        let manifest: Manifest = serde_json::from_slice(&fs::read(base.join("manifest.json"))?)?;
        verify_tree(base, &manifest, &["manifest.json"], false)?;
        Ok((manifest, manifest_file))
    }
    /// Validate an existing private data directory and its exact Frayd marker.
    /// The marker must name this path and a valid installation identity; success
    /// permits data reuse, never deletion of application or user data.
    fn data_receipt(path: &Path) -> Result<DataReceipt> {
        safe_path(path)?;
        private_owned_dir(path)?;
        let metadata = fs::symlink_metadata(path.join(DATA_RECEIPT))?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.mode() & 0o777 != 0o600
        {
            return reject("invalid data ownership marker");
        }
        let receipt: DataReceipt = serde_json::from_slice(&fs::read(path.join(DATA_RECEIPT))?)?;
        if receipt.schema != 1
            || receipt.app != "frayd"
            || receipt.path != path
            || uuid::Uuid::parse_str(&receipt.id).is_err()
        {
            return reject("unknown data ownership");
        }
        Ok(receipt)
    }
    /// Reconstruct program ownership from the private prefix and recorded payload.
    /// Validate the recorded data location without reading its contents or marker;
    /// data may be absent during removal, but redirects and overlap remain refused.
    /// Reject changed/unowned files, invalid locks and any active or uncertain child
    /// marker without mutation. Only removal retry may tolerate missing payload.
    fn load_prefix(prefix: &Path, allow_missing: bool) -> Result<Receipt> {
        safe_path(prefix)?;
        private_owned_dir(prefix)?;
        let record = entry(&prefix.join(RECEIPT), RECEIPT)?;
        if record.kind != Kind::File || record.mode != 0o600 {
            return reject("invalid install receipt");
        }
        let receipt: Receipt = serde_json::from_slice(&fs::read(prefix.join(RECEIPT))?)?;
        if receipt.schema != 1
            || receipt.prefix != prefix
            || !disjoint(prefix, &receipt.data)
            || receipt.manifest_file.path != "manifest.json"
            || receipt.manifest_file.kind != Kind::File
        {
            return reject("invalid prefix/data receipt");
        }
        safe_path(&receipt.data)?;
        if !disjoint(prefix, &prospective_library_path(&receipt.data)?) {
            return reject("resolved prefix/data overlap");
        }
        verify_tree(
            prefix,
            &receipt.manifest,
            &["manifest.json", RECEIPT, LOCK, CHILD],
            allow_missing,
        )?;
        verify_entry(prefix, &receipt.manifest_file, allow_missing)?;
        if prefix.join(CHILD).try_exists()? {
            return reject(
                "active or uncertain child receipt: no automatic process termination/recovery",
            );
        }
        let lock = entry(&prefix.join(LOCK), LOCK)?;
        if lock.kind != Kind::File || lock.mode != 0o600 || lock.size != 0 {
            return reject("invalid lifecycle lock");
        }
        Ok(receipt)
    }
    /// Require the exact data identity before launch; this grants no removal authority.
    /// Missing or invalid data remains a launch refusal even when program ownership
    /// alone would permit uninstall. No directory or marker is created here.
    fn validate_start_data(receipt: &Receipt) -> Result<()> {
        let data = data_receipt(&receipt.data)?;
        if data.id != receipt.data_id {
            return reject("data ownership mismatch");
        }
        Ok(())
    }
    /// Obtain explicit consent through --yes or an affirmative stdin response.
    /// Return false for decline/EOF without persistent effects; callers must not
    /// create files, launch the server or remove payload before a true result.
    fn confirm(yes: bool, action: &str) -> Result<bool> {
        if yes {
            return Ok(true);
        }
        eprint!("{action}? [y/N] ");
        io::stderr().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        Ok(matches!(line.trim(), "y" | "Y"))
    }
    /// Exclusively lock an existing validated lifecycle file without changing bytes.
    /// The returned handle holds authority through supervision/removal; contention
    /// fails rather than terminating a process or guessing ownership from a PID.
    fn locked(prefix: &Path) -> Result<File> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(prefix.join(LOCK))?;
        file.try_lock()
            .map_err(|_| "installation active or lifecycle busy")?;
        Ok(file)
    }
    /// Share one launch exclusion across all prefixes that reuse a data identity.
    fn lock_data(path: &Path) -> Result<File> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path.join(DATA_RECEIPT))?;
        file.try_lock()
            .map_err(|_| "data directory active or lifecycle busy")?;
        Ok(file)
    }
    /// Install a validated bundle into a new prefix with separate app-owned data.
    /// All path, manifest and reuse checks precede consent and creation. Reserve a
    /// private prefix, verify a sibling stage and publish atomically; on failure
    /// roll back only paths created by this attempt and preserve application data.
    fn install(prefix: &Path, data: &Path, bundle: &Path, yes: bool) -> Result<()> {
        safe_path(prefix)?;
        safe_path(data)?;
        safe_path(bundle)?;
        let bundle = fs::canonicalize(bundle)?;
        if !disjoint(prefix, data) || !disjoint(prefix, &bundle) || !disjoint(data, &bundle) {
            return reject("bundle/prefix/data overlap");
        }
        if prefix.try_exists()? {
            return reject("existing prefix refused; upgrades are not supported");
        }
        if !prefix.parent().ok_or("missing prefix parent")?.is_dir()
            || !data.parent().ok_or("missing data parent")?.is_dir()
        {
            return reject("target parents must already exist");
        }
        let (manifest, manifest_file) = load_bundle(&bundle)?;
        let reuse_data = data.try_exists()?;
        let data_record = if reuse_data {
            data_receipt(data)?
        } else {
            DataReceipt {
                schema: 1,
                app: "frayd".into(),
                path: data.into(),
                id: uuid::Uuid::new_v4().to_string(),
            }
        };
        if !confirm(
            yes,
            "Install new owned prefix and preserve all application data",
        )? {
            return Ok(());
        }
        // Reserve only a NEW prefix, then atomically rename a complete sibling stage over
        // our own empty reservation. A nonempty replacement causes rename to fail.
        let signals = InstallSignals::block()?;
        signals.checkpoint()?;
        private_dir(prefix)?;
        let reservation = fs::symlink_metadata(prefix).map_err(|error| {
            format!(
                "prefix metadata failed: {error}; unqualified prefix retained at {}; inspect before recovery",
                prefix.display()
            )
        })?;
        let stage = prefix
            .parent()
            .ok_or("missing parent")?
            .join(format!(".frayd-stage-{}", uuid::Uuid::new_v4()));
        if let Err(error) = private_dir(&stage) {
            remove_reservation(prefix, &reservation).map_err(|cleanup| {
                format!("stage creation failed: {error}; prefix reservation retained: {cleanup}")
            })?;
            return Err(error);
        }
        let stage_identity = match CreatedPath::record(stage.clone()) {
            Ok(identity) => identity,
            Err(error) => {
                return Err(retain_unqualified_stage(
                    prefix,
                    &reservation,
                    &stage,
                    error,
                ))
            }
        };
        let mut created_data = None;
        let mut created_files = Vec::new();
        let mut created_dirs = Vec::new();
        let result = (|| -> Result<()> {
            signals.checkpoint()?;
            let mut dirs: Vec<_> = manifest
                .entries
                .iter()
                .filter(|e| e.kind == Kind::Dir)
                .collect();
            dirs.sort_by_key(|e| e.path.matches('/').count());
            for item in dirs {
                signals.checkpoint()?;
                private_dir(&stage.join(&item.path))?;
                created_dirs.push(CreatedPath::record(stage.join(&item.path))?);
            }
            for item in manifest
                .entries
                .iter()
                .filter(|e| e.kind == Kind::File)
                .chain(std::iter::once(&manifest_file))
            {
                signals.checkpoint()?;
                let mut source = File::open(bundle.join(&item.path))?;
                let mut dest = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(stage.join(&item.path))?;
                created_files.push(CreatedPath::record(stage.join(&item.path))?);
                copy_install_file(&mut source, &mut dest, &signals)?;
                fs::set_permissions(
                    stage.join(&item.path),
                    fs::Permissions::from_mode(item.mode),
                )?;
            }
            // Directory modes are restored only after all private staging writes finish.
            for item in manifest.entries.iter().filter(|e| e.kind == Kind::Dir) {
                signals.checkpoint()?;
                fs::set_permissions(
                    stage.join(&item.path),
                    fs::Permissions::from_mode(item.mode),
                )?;
            }
            signals.checkpoint()?;
            verify_tree(&stage, &manifest, &["manifest.json"], false)?;
            signals.checkpoint()?;
            verify_entry(&stage, &manifest_file, false)?;
            if !reuse_data {
                private_dir(data)?;
                created_data = Some(fs::symlink_metadata(data)?);
                private_file(
                    &data.join(DATA_RECEIPT),
                    &serde_json::to_vec_pretty(&data_record)?,
                )?;
            }
            let current_data = data_receipt(data)?;
            if current_data.id != data_record.id {
                return reject("data ownership changed during installation");
            }
            private_file(&stage.join(LOCK), b"")?;
            created_files.push(CreatedPath::record(stage.join(LOCK))?);
            let receipt = Receipt {
                schema: 1,
                prefix: prefix.into(),
                data: data.into(),
                data_id: data_record.id,
                manifest,
                manifest_file,
            };
            private_file(&stage.join(RECEIPT), &serde_json::to_vec_pretty(&receipt)?)?;
            created_files.push(CreatedPath::record(stage.join(RECEIPT))?);
            let current = fs::symlink_metadata(prefix)?;
            if !current.is_dir()
                || current.dev() != reservation.dev()
                || current.ino() != reservation.ino()
                || fs::read_dir(prefix)?.next().is_some()
            {
                return reject("prefix reservation changed; refusing publication");
            }
            publish_install(&stage, prefix, &signals)?;
            Ok(())
        })();
        if let Err(error) = result {
            let error = signals.checkpoint().err().unwrap_or(error);
            // Roll back only paths created in this attempt; never recurse through an
            // unexpected tree. Failure retains exact paths and reports the original error.
            let retained = rollback_install(
                prefix,
                data,
                &stage_identity,
                &reservation,
                created_data.as_ref(),
                &created_files,
                &created_dirs,
            );
            return Err(format!(
                "install failed: {error}; retained paths: {retained:?}; data is never deleted"
            )
            .into());
        }
        println!("Installed owned prefix; data is preserved independently.");
        Ok(())
    }
    /// Without a stage identity, preserve it and clean only the qualified empty reservation.
    fn retain_unqualified_stage(
        prefix: &Path,
        reservation: &fs::Metadata,
        stage: &Path,
        error: Box<dyn std::error::Error>,
    ) -> Box<dyn std::error::Error> {
        let mut retained = vec![stage.display().to_string()];
        if remove_reservation(prefix, reservation).is_err() {
            retained.push(prefix.display().to_string());
        }
        format!(
            "stage metadata failed: {error}; retained paths: {retained:?}; inspect before recovery"
        )
        .into()
    }
    /// Remove an empty reservation only while it still has the recorded identity.
    fn remove_reservation(prefix: &Path, reservation: &fs::Metadata) -> Result<()> {
        let current = fs::symlink_metadata(prefix)?;
        if !current.is_dir()
            || current.dev() != reservation.dev()
            || current.ino() != reservation.ino()
        {
            return reject("prefix reservation changed; refusing cleanup");
        }
        fs::remove_dir(prefix)?;
        Ok(())
    }
    /// Clean only attempt-owned paths; uncertainty retains paths without hiding the failure.
    fn rollback_install(
        prefix: &Path,
        data: &Path,
        stage: &CreatedPath,
        reservation: &fs::Metadata,
        created_data: Option<&fs::Metadata>,
        created_files: &[CreatedPath],
        created_dirs: &[CreatedPath],
    ) -> Vec<String> {
        let mut retained = Vec::new();
        if let Some(identity) = created_data {
            match data.join(DATA_RECEIPT).try_exists() {
                // Preserve marked or uncertain data and report its exact path.
                Ok(false) if remove_reservation(data, identity).is_ok() => (),
                _ => retained.push(data.display().to_string()),
            }
        }
        for file in created_files.iter().rev() {
            if file.remove().is_err() {
                retained.push(file.path.display().to_string());
            }
        }
        for directory in created_dirs.iter().rev() {
            if directory.remove().is_err() {
                retained.push(directory.path.display().to_string());
            }
        }
        if stage.remove().is_err() {
            retained.push(stage.path.display().to_string());
        }
        if remove_reservation(prefix, reservation).is_err() {
            retained.push(prefix.display().to_string());
        }
        retained
    }
    fn require_child_wait_custody() -> Result<()> {
        // Do not normalize inherited process-global policy. Ignored SIGCHLD,
        // SA_NOCLDWAIT or an unknown handler could reap/release the leader PID.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(libc::SIGCHLD, std::ptr::null(), &mut action) != 0 {
                return Err(io::Error::last_os_error().into());
            }
            if action.sa_sigaction != libc::SIG_DFL || action.sa_flags & libc::SA_NOCLDWAIT != 0 {
                return reject("unsupported SIGCHLD disposition; start requires default child wait custody; no child spawned");
            }
        }
        Ok(())
    }

    /// Observe exit without reaping: the leader PID reserves its process-group ID
    /// until all nonzero group signals have completed. Never call try_wait here.
    fn child_exited_unreaped(pid: libc::pid_t) -> Result<bool> {
        // SAFETY: valid zero-initialized siginfo and a direct, unreaped child PID.
        unsafe {
            let mut info: libc::siginfo_t = std::mem::zeroed();
            if libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
            ) != 0
            {
                let error = io::Error::last_os_error();
                return Err(error.into());
            }
            Ok(info.si_signo == libc::SIGCHLD)
        }
    }
    fn owned_group_lookup_authorized(
        pid: libc::pid_t,
        lookup: std::result::Result<libc::pid_t, i32>,
        exited_unreaped: bool,
    ) -> bool {
        match lookup {
            Ok(group) => group == pid,
            Err(error) => error == libc::ESRCH && exited_unreaped,
        }
    }
    #[cfg(any(target_os = "macos", test))]
    fn macos_exited_group_signal_progress(
        error: io::Error,
        exited_unreaped: Result<bool>,
    ) -> Result<()> {
        if error.raw_os_error() != Some(libc::EPERM) {
            return Err(error.into());
        }
        // Darwin may report EPERM with only exited group members remaining.
        // Fresh wait custody permits progress, never a claim of group absence.
        // The caller still reaps and requires the final zero probe's ESRCH.
        let exited = exited_unreaped.map_err(|error| {
            format!("owned child wait custody uncertain: {error}; child receipts retained")
        })?;
        if !exited {
            return Err(error.into());
        }
        Ok(())
    }

    fn group_probe_absent(probe: io::Result<()>) -> Result<bool> {
        match probe {
            Ok(()) => Ok(false),
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(true),
            Err(error) => Err(format!(
                "child group cleanup uncertain: {error}; child receipts retained"
            )
            .into()),
        }
    }

    async fn wait_for_group_absence(
        mut probe: impl FnMut() -> io::Result<()>,
        retry_permission_denied: bool,
        deadline: tokio::time::Instant,
    ) -> Result<()> {
        // Only ESRCH establishes absence. The caller has already reaped and
        // must never send another nonzero signal to this reusable group ID.
        loop {
            let result = probe();
            // Darwin can report EPERM while only exited group members remain.
            // This is unresolved, never absence: require ESRCH within the same deadline.
            let absent = match result {
                Err(error)
                    if retry_permission_denied && error.raw_os_error() == Some(libc::EPERM) =>
                {
                    false
                }
                result => group_probe_absent(result)?,
            };
            if absent {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return reject("child group still exists after cleanup; child receipts retained");
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    fn owned_group_signal(pid: libc::pid_t, signal: i32) -> Result<()> {
        // ECHILD rejects signalling before any numeric PID/PGID lookup or kill.
        // WNOWAIT also checks custody immediately before each nonzero signal.
        child_exited_unreaped(pid).map_err(|error| {
            format!("owned child wait custody uncertain: {error}; child receipts retained")
        })?;
        // Always target the spawn-owned group ID, never a lookup-supplied ID.
        if pid <= 1 || pid == unsafe { libc::getpgrp() } {
            return reject("owned child group identity uncertain; child receipts retained");
        }
        let group = unsafe { libc::getpgid(pid) };
        let lookup = if group < 0 {
            Err(io::Error::last_os_error()
                .raw_os_error()
                .unwrap_or(libc::EIO))
        } else {
            Ok(group)
        };
        // Darwin may hide an exited process from getpgid while WNOWAIT still
        // pins that owned child's PID. Confirm the exited state afresh.
        let exited_unreaped = if lookup == Err(libc::ESRCH) {
            child_exited_unreaped(pid).map_err(|error| {
                format!("owned child wait custody uncertain: {error}; child receipts retained")
            })?
        } else {
            false
        };
        if !owned_group_lookup_authorized(pid, lookup, exited_unreaped) {
            return reject("owned child group identity uncertain; child receipts retained");
        }
        if unsafe { libc::killpg(pid, signal) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                #[cfg(target_os = "macos")]
                if error.raw_os_error() == Some(libc::EPERM) {
                    return macos_exited_group_signal_progress(error, child_exited_unreaped(pid));
                }
                return Err(error.into());
            }
            // Missing eligible group members is progress, not removal authority.
            // Keep child custody and still require reap plus the zero-only probe.
            child_exited_unreaped(pid).map_err(|error| {
                format!("owned child wait custody uncertain: {error}; child receipts retained")
            })?;
        }
        Ok(())
    }
    async fn stop_owned_group(child: &mut std::process::Child) -> Result<std::process::ExitStatus> {
        let pid = libc::pid_t::try_from(child.id())?;
        owned_group_signal(pid, libc::SIGTERM)?;
        // Keep the leader unreaped through both signals, including when TERM
        // already exited it. Descendants may ignore TERM or outlive that leader.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        owned_group_signal(pid, libc::SIGKILL)?;
        let reap_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while !child_exited_unreaped(pid)? {
            if tokio::time::Instant::now() >= reap_deadline {
                return reject("owned child did not exit after cleanup; child receipts retained");
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let status = child.wait()?;
        // No further nonzero signal may use this now-reusable ID. A surviving,
        // permission-denied or reused group conservatively retains both receipts.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        wait_for_group_absence(
            || {
                // SAFETY: signal zero only probes existence and cannot terminate a process.
                if unsafe { libc::killpg(pid, 0) } == 0 {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error())
                }
            },
            cfg!(target_os = "macos"),
            deadline,
        )
        .await?;
        Ok(status)
    }

    #[cfg(any(target_os = "macos", test))]
    fn is_null_device_output_error(descriptor: libc::c_int, error: &io::Error) -> Result<bool> {
        if error.raw_os_error() != Some(libc::ENODEV) {
            return Ok(false);
        }
        // Darwin's memory-device ioctl returns ENODEV for /dev/null. Admit only
        // that character device, not every descriptor with a failed terminal query.
        let mut descriptor_stat: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: fstat initializes valid storage and does not take fd ownership.
        if unsafe { libc::fstat(descriptor, &mut descriptor_stat) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        let mut null_stat: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: the path is NUL-terminated and stat initializes valid storage.
        if unsafe { libc::stat(c"/dev/null".as_ptr(), &mut null_stat) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(descriptor_stat.st_mode & libc::S_IFMT == libc::S_IFCHR
            && null_stat.st_mode & libc::S_IFMT == libc::S_IFCHR
            && descriptor_stat.st_rdev == null_stat.st_rdev)
    }

    fn require_supported_output_descriptor(descriptor: libc::c_int) -> Result<()> {
        // SAFETY: tcgetattr initializes a valid termios; ENOTTY accepts pipes/files.
        let mut attributes: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(descriptor, &mut attributes) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ENOTTY) {
                return Ok(());
            }
            #[cfg(target_os = "macos")]
            if is_null_device_output_error(descriptor, &error)? {
                return Ok(());
            }
            return Err(error.into());
        }
        if attributes.c_lflag & libc::TOSTOP != 0 {
            return reject("output terminal has TOSTOP enabled; disable TOSTOP or redirect both stdout and stderr before start");
        }
        Ok(())
    }

    fn require_supported_output() -> Result<()> {
        for descriptor in [libc::STDOUT_FILENO, libc::STDERR_FILENO] {
            require_supported_output_descriptor(descriptor)?;
        }
        Ok(())
    }
    /// Launch the unchanged owned server with explicit private runtime paths.
    /// Validate prefix/data and hold the lifecycle lock before spawning. Normal
    /// SIGINT/SIGTERM/SIGHUP/SIGQUIT stops the owned server process group; a crash or
    /// SIGKILL leaves its uncertainty marker to block later start/removal safely.
    fn start(prefix: &Path, port: u16) -> Result<()> {
        require_supported_output()?;
        require_child_wait_custody()?;
        // Validate the lock file before opening it, then read the receipt under the lock.
        load_prefix(prefix, false)?;
        let _lock = locked(prefix)?;
        let receipt = load_prefix(prefix, false)?;
        validate_start_data(&receipt)?;
        let _data_lock = lock_data(&receipt.data)?;
        validate_start_data(&receipt)?;
        if fs::symlink_metadata(receipt.data.join(CHILD)).is_ok() {
            return Err(format!("active or uncertain data child receipt at {}; verify the old server has stopped before operator recovery; no automatic termination or marker removal", receipt.data.join(CHILD).display()).into());
        }
        for found in WalkDir::new(&receipt.data).follow_links(false) {
            let found = found?;
            if found.file_type().is_symlink()
                || (!found.file_type().is_file() && !found.file_type().is_dir())
            {
                return reject("runtime data contains a redirect/special file");
            }
            if found.file_type().is_file() && found.metadata()?.nlink() != 1 {
                return reject("runtime data contains a hardlink");
            }
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            // Register before child creation. The server leads its own group; the
            // helper remains outside it. A crash still leaves fail-closed receipts.
            let mut interrupt =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            let mut hangup =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?;
            let mut quit = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::quit())?;
            let mut child_record =
                private_file(&prefix.join(CHILD), b"{\"state\":\"spawning-or-unknown\"}")?;
            let mut data_child_record = match private_file(
                &receipt.data.join(CHILD),
                b"{\"state\":\"spawning-or-unknown\"}",
            ) {
                Ok(file) => file,
                Err(error) => {
                    remove_private_file(&prefix.join(CHILD), &child_record)?;
                    return Err(error);
                }
            };
            let child = std::process::Command::new(prefix.join("frayd-server"))
                .current_dir(&receipt.data)
                .env("FRAYD_DB", receipt.data.join("frayd.db"))
                .env("FRAYD_THUMBS", receipt.data.join("thumbs"))
                .env("FRAYD_TRANSCODES", receipt.data.join("transcodes"))
                .env("FRAYD_CLIENT_DIR", prefix.join("client"))
                .env("FRAYD_BIND", "127.0.0.1")
                .env("FRAYD_PORT", port.to_string())
                .stdin(std::process::Stdio::null())
                .process_group(0)
                .spawn();
            let mut child = match child {
                Ok(child) => child,
                Err(error) => {
                    remove_private_file(&receipt.data.join(CHILD), &data_child_record)?;
                    remove_private_file(&prefix.join(CHILD), &child_record)?;
                    return Err(error.into());
                }
            };
            record_child(&mut child_record, Some(child.id()))?;
            record_child(&mut data_child_record, Some(child.id()))?;
            let pid = libc::pid_t::try_from(child.id())?;
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(20));
            let interrupted = loop {
                tokio::select! {
                    biased;
                    _ = interrupt.recv() => break true,
                    _ = terminate.recv() => break true,
                    _ = hangup.recv() => break true,
                    _ = quit.recv() => break true,
                    _ = tick.tick() => {
                        if child_exited_unreaped(pid)? { break false; }
                    },
                }
            };
            let status = stop_owned_group(&mut child).await?;
            let interrupted =
                interrupted || matches!(status.signal(), Some(1) | Some(2) | Some(3) | Some(15));
            remove_private_file(&receipt.data.join(CHILD), &data_child_record)?;
            remove_private_file(&prefix.join(CHILD), &child_record)?;
            if !status.success() && !interrupted {
                return Err(format!("server exited with {status}; data preserved").into());
            }
            Ok(())
        })
    }
    /// Preflight final prefix removal without touching its payload.
    /// Require an owner-writable parent or a sticky publicly writable parent so a
    /// known permission failure cannot consume payload before retaining retry state.
    fn removal_parent_writable(prefix: &Path) -> Result<()> {
        let parent = fs::metadata(prefix.parent().ok_or("missing prefix parent")?)?;
        let owner = fs::metadata(prefix)?.uid();
        let writable = if parent.uid() == owner {
            parent.mode() & 0o200 != 0
        } else {
            parent.mode() & 0o1002 == 0o1002
        };
        if !writable {
            return reject("prefix parent is not safely writable; no removal attempted");
        }
        Ok(())
    }
    /// Remove only unchanged recorded payload after validation and consent.
    /// Hold the lifecycle lock, permit missing owned files for interrupted retries
    /// and remove directories only when empty. Preserve all data; retain or restore
    /// receipt metadata on cleanup failure when the filesystem permits recovery.
    fn uninstall(prefix: &Path, yes: bool) -> Result<()> {
        load_prefix(prefix, true)?;
        removal_parent_writable(prefix)?;
        let _lock = locked(prefix)?;
        let receipt = load_prefix(prefix, true)?;
        if !confirm(
            yes,
            &format!(
                "Remove unchanged owned installation version {} at {}; keep all data at {}",
                receipt.manifest.version,
                prefix.display(),
                receipt.data.display()
            ),
        )? {
            return Ok(());
        }
        for item in receipt
            .manifest
            .entries
            .iter()
            .filter(|e| e.kind == Kind::File)
            .chain(std::iter::once(&receipt.manifest_file))
        {
            match fs::remove_file(prefix.join(&item.path)) {
                Ok(()) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => {
                    return Err(
                        format!("removal incomplete; receipt retained for retry: {e}").into(),
                    )
                }
            }
        }
        let mut dirs: Vec<_> = receipt
            .manifest
            .entries
            .iter()
            .filter(|e| e.kind == Kind::Dir)
            .collect();
        dirs.sort_by_key(|e| std::cmp::Reverse(e.path.matches('/').count()));
        for item in dirs {
            match fs::remove_dir(prefix.join(&item.path)) {
                Ok(()) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => {
                    return Err(
                        format!("directory retained; retry using an extracted helper: {e}").into(),
                    )
                }
            }
        }
        let receipt_bytes = fs::read(prefix.join(RECEIPT))?;
        let identity = fs::symlink_metadata(prefix)?;
        let cleanup = (|| -> io::Result<()> {
            fs::remove_file(prefix.join(RECEIPT))?;
            fs::remove_file(prefix.join(LOCK))?;
            fs::remove_dir(prefix)
        })();
        if let Err(error) = cleanup {
            // Reconstitute removal authority when final directory cleanup fails.
            // Never replace an existing metadata path or adopt a substituted prefix.
            let current = fs::symlink_metadata(prefix).map_err(|metadata| {
                format!(
                    "final removal failed: {error}; cannot inspect retained prefix {}: {metadata}",
                    prefix.display()
                )
            })?;
            if current.is_dir()
                && current.dev() == identity.dev()
                && current.ino() == identity.ino()
            {
                for (name, bytes) in [(LOCK, b"".as_slice()), (RECEIPT, receipt_bytes.as_slice())] {
                    if !prefix.join(name).try_exists().map_err(|metadata| {
                        format!("final removal failed: {error}; cannot inspect recovery metadata {}: {metadata}", prefix.join(name).display())
                    })? {
                        private_file(&prefix.join(name), bytes).map_err(|restore| format!("final removal \
                            failed: {error}; metadata recovery failed at {}: {restore}; retained prefix \
                            requires inspection", prefix.display()))?;
                    }
                }
            }
            return Err(format!(
                "final removal failed: {error}; retained prefix {} and recovery metadata require retry/inspection",
                prefix.display()
            )
            .into());
        }
        println!(
            "Removed owned installation; all application data preserved at {}",
            receipt.data.display()
        );
        Ok(())
    }

    /// Parse the explicit native CLI and dispatch exactly one supported lifecycle.
    /// Reject unknown, duplicate, purge and malformed arguments before dispatch;
    /// individual operations enforce path, ownership and confirmation boundaries.
    fn run() -> Result<()> {
        let mut args = std::env::args().skip(1);
        let command = args
            .next()
            .ok_or("expected --build-info, install, start or uninstall")?;
        if command == "--build-info" {
            if args.next().is_some() {
                return reject("--build-info accepts no additional arguments");
            }
            let mut identity = super::build_info::metadata();
            identity["role"] = "frayd-native".into();
            println!("{identity}");
            return Ok(());
        }
        let mut flags = BTreeMap::new();
        let mut yes = false;
        while let Some(flag) = args.next() {
            if flag == "--yes" && !yes && command != "start" {
                yes = true;
                continue;
            }
            let allowed = match command.as_str() {
                "install" => ["--prefix", "--data-dir", "--bundle"].as_slice(),
                "start" => ["--prefix", "--port"].as_slice(),
                "uninstall" => ["--prefix"].as_slice(),
                _ => return reject("unknown command"),
            };
            if !allowed.contains(&flag.as_str()) || flags.contains_key(&flag) {
                return reject(
                    "unknown or duplicate flag; purge and legacy targets are unsupported",
                );
            }
            flags.insert(flag, args.next().ok_or("flag value required")?);
        }
        let prefix = Path::new(flags.get("--prefix").ok_or("--prefix is required")?);
        match command.as_str() {
            "install" => install(
                prefix,
                Path::new(flags.get("--data-dir").ok_or("--data-dir required")?),
                Path::new(flags.get("--bundle").ok_or("--bundle required")?),
                yes,
            ),
            "start" => {
                let port = flags
                    .get("--port")
                    .map(|p| p.parse::<u16>())
                    .transpose()?
                    .unwrap_or(8484);
                if port == 0 {
                    return reject("port must be 1..65535");
                }
                start(prefix, port)
            }
            "uninstall" => uninstall(prefix, yes),
            _ => reject("unknown command"),
        }
    }
    /// Exit 0 on success or a declined confirmation; print the error and exit 2 otherwise.
    pub fn main() -> ExitCode {
        match run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("frayd-native: {error}");
                ExitCode::from(2)
            }
        }
    }
    #[cfg(test)]
    mod rollback_tests {
        use super::*;
        use std::os::unix::fs::symlink;

        #[test]
        fn output_admission_accepts_redirects_and_rejects_closed_descriptors() {
            use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

            let null = OpenOptions::new().write(true).open("/dev/null").unwrap();
            require_supported_output_descriptor(null.as_raw_fd()).unwrap();
            let file = File::open(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
            require_supported_output_descriptor(file.as_raw_fd()).unwrap();
            let mut pipe = [-1; 2];
            // SAFETY: valid fd storage; OwnedFd closes each newly created fd once.
            assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
            let _reader = unsafe { OwnedFd::from_raw_fd(pipe[0]) };
            let writer = unsafe { OwnedFd::from_raw_fd(pipe[1]) };
            require_supported_output_descriptor(writer.as_raw_fd()).unwrap();
            let error = require_supported_output_descriptor(-1).unwrap_err();
            assert_eq!(
                error.downcast_ref::<io::Error>().unwrap().raw_os_error(),
                Some(libc::EBADF)
            );
        }

        #[test]
        fn output_admission_darwin_exception_requires_null_identity_and_enodev() {
            use std::os::fd::AsRawFd;

            let enodev = io::Error::from_raw_os_error(libc::ENODEV);
            let null = File::open("/dev/null").unwrap();
            assert!(is_null_device_output_error(null.as_raw_fd(), &enodev).unwrap());
            let file = File::open(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
            assert!(!is_null_device_output_error(file.as_raw_fd(), &enodev).unwrap());
            let zero = File::open("/dev/zero").unwrap();
            assert!(!is_null_device_output_error(zero.as_raw_fd(), &enodev).unwrap());
            for errno in [libc::EBADF, libc::EIO, libc::EINVAL] {
                assert!(!is_null_device_output_error(
                    null.as_raw_fd(),
                    &io::Error::from_raw_os_error(errno)
                )
                .unwrap());
            }
            let error = is_null_device_output_error(-1, &enodev).unwrap_err();
            assert_eq!(
                error.downcast_ref::<io::Error>().unwrap().raw_os_error(),
                Some(libc::EBADF)
            );
        }

        #[test]
        fn output_admission_preserves_real_terminal_tostop_guard() {
            use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

            let mut master = -1;
            let mut slave = -1;
            // SAFETY: valid fd outputs; null termios/winsize request system defaults.
            assert_eq!(
                unsafe {
                    libc::openpty(
                        &mut master,
                        &mut slave,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    )
                },
                0
            );
            let _master = unsafe { OwnedFd::from_raw_fd(master) };
            let slave = unsafe { OwnedFd::from_raw_fd(slave) };
            let mut attributes: libc::termios = unsafe { std::mem::zeroed() };
            assert_eq!(
                unsafe { libc::tcgetattr(slave.as_raw_fd(), &mut attributes) },
                0
            );
            attributes.c_lflag |= libc::TOSTOP;
            assert_eq!(
                unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &attributes) },
                0
            );
            assert!(require_supported_output_descriptor(slave.as_raw_fd())
                .unwrap_err()
                .to_string()
                .contains("TOSTOP"));
            let enodev = io::Error::from_raw_os_error(libc::ENODEV);
            assert!(!is_null_device_output_error(slave.as_raw_fd(), &enodev).unwrap());
            attributes.c_lflag &= !libc::TOSTOP;
            assert_eq!(
                unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &attributes) },
                0
            );
            require_supported_output_descriptor(slave.as_raw_fd()).unwrap();
        }

        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).expect("remove owned rollback fixture");
            }
        }

        fn scenario(marker: &str, missing_prefix: bool) {
            let fixture = Fixture(
                Path::new("/var/tmp")
                    .join(format!("frayd-native-rollback-{}", uuid::Uuid::new_v4())),
            );
            private_dir(&fixture.0).unwrap();
            let prefix = fixture.0.join("prefix");
            let data = fixture.0.join("data");
            let stage = fixture.0.join("stage");
            for path in [&prefix, &data, &stage] {
                private_dir(path).unwrap();
            }
            let reservation = fs::symlink_metadata(&prefix).unwrap();
            let directory = stage.join("payload");
            private_dir(&directory).unwrap();
            let file = directory.join("file");
            fs::write(&file, "owned payload").unwrap();
            match marker {
                "marked" => fs::write(data.join(DATA_RECEIPT), "owned marker").unwrap(),
                "uncertain" => symlink(DATA_RECEIPT, data.join(DATA_RECEIPT)).unwrap(),
                "nonempty" => fs::write(data.join("unexpected"), "preserve").unwrap(),
                "empty" => (),
                _ => panic!("unknown fixture"),
            }
            if missing_prefix {
                fs::remove_dir(&prefix).unwrap();
            }
            let retained = rollback_install(
                &prefix,
                &data,
                &CreatedPath::record(stage.clone()).unwrap(),
                &reservation,
                Some(&fs::symlink_metadata(&data).unwrap()),
                &[CreatedPath::record(file).unwrap()],
                &[CreatedPath::record(directory).unwrap()],
            );
            assert!(!stage.exists(), "stage cleanup skipped");
            assert!(!prefix.exists(), "empty reservation cleanup skipped");
            if marker == "empty" {
                assert!(!data.exists());
                assert!(retained.is_empty());
            } else {
                assert!(data.is_dir());
                assert!(retained.contains(&data.display().to_string()));
                match marker {
                    "marked" => {
                        assert_eq!(fs::read(data.join(DATA_RECEIPT)).unwrap(), b"owned marker")
                    }
                    "uncertain" => assert_eq!(
                        fs::read_link(data.join(DATA_RECEIPT)).unwrap(),
                        Path::new(DATA_RECEIPT)
                    ),
                    "nonempty" => {
                        assert_eq!(fs::read(data.join("unexpected")).unwrap(), b"preserve")
                    }
                    _ => unreachable!(),
                }
            }
            if missing_prefix {
                assert!(retained.contains(&prefix.display().to_string()));
            }
        }

        static CHILD_POLICY_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
        struct SavedChildPolicy(libc::sigaction);
        impl Drop for SavedChildPolicy {
            fn drop(&mut self) {
                assert_eq!(
                    unsafe { libc::sigaction(libc::SIGCHLD, &self.0, std::ptr::null_mut()) },
                    0
                );
            }
        }

        #[test]
        fn start_rejects_ignored_and_no_child_wait_policy_before_paths_or_spawn() {
            let _serial = CHILD_POLICY_TEST.lock().unwrap();
            for (handler, flags) in [(libc::SIG_IGN, 0), (libc::SIG_DFL, libc::SA_NOCLDWAIT)] {
                unsafe {
                    let mut previous = std::mem::zeroed();
                    let mut action: libc::sigaction = std::mem::zeroed();
                    action.sa_sigaction = handler;
                    action.sa_flags = flags;
                    libc::sigemptyset(&mut action.sa_mask);
                    assert_eq!(libc::sigaction(libc::SIGCHLD, &action, &mut previous), 0);
                    let _restore = SavedChildPolicy(previous);
                    let missing = Path::new("/var/tmp")
                        .join(format!("frayd-never-created-{}", uuid::Uuid::new_v4()));
                    let error = start(&missing, 18484).unwrap_err().to_string();
                    assert!(error.contains("unsupported SIGCHLD disposition"), "{error}");
                    assert!(!missing.exists());
                }
            }
        }

        #[test]
        fn missing_group_lookup_requires_confirmed_unreaped_exit() {
            // Inject Darwin's lookup result only; no fake kernel or numeric PID signal.
            for (lookup, exited, authorized) in [
                (Ok(42), false, true),
                (Ok(42), true, true),
                (Err(libc::ESRCH), true, true),
                (Err(libc::ESRCH), false, false),
                (Ok(43), false, false),
                (Ok(43), true, false),
                (Err(libc::EPERM), true, false),
                (Err(libc::ECHILD), true, false),
                (Err(libc::EIO), true, false),
            ] {
                assert_eq!(
                    owned_group_lookup_authorized(42, lookup, exited),
                    authorized,
                    "lookup={lookup:?}, exited={exited}"
                );
            }
        }

        #[test]
        fn macos_group_eperm_progress_requires_fresh_confirmed_exit() {
            let permission_error = || io::Error::from_raw_os_error(libc::EPERM);
            macos_exited_group_signal_progress(permission_error(), Ok(true)).unwrap();
            let error =
                macos_exited_group_signal_progress(permission_error(), Ok(false)).unwrap_err();
            assert_eq!(
                error.downcast_ref::<io::Error>().unwrap().raw_os_error(),
                Some(libc::EPERM)
            );
            for errno in [libc::ECHILD, libc::EIO] {
                let error = macos_exited_group_signal_progress(
                    permission_error(),
                    Err(io::Error::from_raw_os_error(errno).into()),
                )
                .unwrap_err()
                .to_string();
                assert!(error.contains("wait custody uncertain"), "{error}");
                assert!(error.contains("child receipts retained"), "{error}");
            }
            let error = macos_exited_group_signal_progress(
                io::Error::from_raw_os_error(libc::EIO),
                Ok(true),
            )
            .unwrap_err();
            assert_eq!(
                error.downcast_ref::<io::Error>().unwrap().raw_os_error(),
                Some(libc::EIO)
            );
        }

        #[test]
        fn group_probe_never_clears_live_or_permission_denied_groups() {
            assert!(!group_probe_absent(Ok(())).unwrap());
            assert!(group_probe_absent(Err(io::Error::from_raw_os_error(libc::ESRCH))).unwrap());
            for errno in [libc::EPERM, libc::EIO, libc::EINVAL] {
                let error = group_probe_absent(Err(io::Error::from_raw_os_error(errno)))
                    .unwrap_err()
                    .to_string();
                assert!(error.contains("child group cleanup uncertain"), "{error}");
                assert!(error.contains("child receipts retained"), "{error}");
            }
        }

        #[tokio::test]
        async fn macos_group_probe_retries_until_confirmed_absence() {
            let mut results = [Err(libc::EPERM), Ok(()), Err(libc::ESRCH)].into_iter();
            let mut calls = 0;
            wait_for_group_absence(
                || {
                    calls += 1;
                    results
                        .next()
                        .expect("probe after absence")
                        .map_err(io::Error::from_raw_os_error)
                },
                true,
                tokio::time::Instant::now() + std::time::Duration::from_secs(1),
            )
            .await
            .unwrap();
            assert_eq!(calls, 3, "only ESRCH may establish absence");
        }

        #[tokio::test]
        async fn unresolved_group_probe_keeps_its_original_deadline() {
            for permission_denied in [false, true] {
                let mut calls = 0;
                let error = wait_for_group_absence(
                    || {
                        calls += 1;
                        if permission_denied {
                            Err(io::Error::from_raw_os_error(libc::EPERM))
                        } else {
                            Ok(())
                        }
                    },
                    true,
                    tokio::time::Instant::now(),
                )
                .await
                .unwrap_err()
                .to_string();
                assert_eq!(calls, 1, "expired deadline must never be renewed");
                assert!(
                    error.contains("child group still exists after cleanup"),
                    "{error}"
                );
                assert!(error.contains("child receipts retained"), "{error}");
            }
        }

        #[tokio::test]
        async fn group_probe_other_errors_fail_without_retry() {
            for retry_permission_denied in [false, true] {
                for errno in [libc::EPERM, libc::EIO, libc::EINVAL] {
                    if retry_permission_denied && errno == libc::EPERM {
                        continue;
                    }
                    let mut calls = 0;
                    let error = wait_for_group_absence(
                        || {
                            calls += 1;
                            Err(io::Error::from_raw_os_error(errno))
                        },
                        retry_permission_denied,
                        tokio::time::Instant::now() + std::time::Duration::from_secs(1),
                    )
                    .await
                    .unwrap_err()
                    .to_string();
                    assert_eq!(calls, 1, "unexpected error must fail immediately");
                    assert!(error.contains("child group cleanup uncertain"), "{error}");
                    assert!(error.contains("child receipts retained"), "{error}");
                }
            }
        }

        #[test]
        fn reaped_child_has_no_signal_authority() {
            let _serial = CHILD_POLICY_TEST.lock().unwrap();
            let mut child = std::process::Command::new("/bin/sh")
                .args(["-c", "exit 0"])
                .process_group(0)
                .spawn()
                .unwrap();
            let pid = libc::pid_t::try_from(child.id()).unwrap();
            assert!(child.wait().unwrap().success());
            // This only proves ECHILD rejection, never forces or signals PID reuse.
            let error = owned_group_signal(pid, libc::SIGTERM)
                .unwrap_err()
                .to_string();
            assert!(error.contains("wait custody uncertain"), "{error}");
        }

        #[test]
        fn cancellation_before_publication_preserves_both_directories_and_restores_mask() {
            for cancellation in INSTALL_SIGNALS {
                let fixture = Fixture(
                    Path::new("/var/tmp")
                        .join(format!("frayd-native-cancel-{}", uuid::Uuid::new_v4())),
                );
                private_dir(&fixture.0).unwrap();
                let stage = fixture.0.join("stage");
                let prefix = fixture.0.join("prefix");
                private_dir(&stage).unwrap();
                private_dir(&prefix).unwrap();
                let previous = unsafe {
                    let mut mask = std::mem::zeroed();
                    assert_eq!(
                        libc::pthread_sigmask(libc::SIG_SETMASK, std::ptr::null(), &mut mask),
                        0
                    );
                    mask
                };
                {
                    let signals = InstallSignals::block().unwrap();
                    // Thread-targeted signals cannot interrupt another parallel Rust test.
                    assert_eq!(
                        unsafe { libc::pthread_kill(libc::pthread_self(), cancellation) },
                        0
                    );
                    let error = publish_install(&stage, &prefix, &signals)
                        .unwrap_err()
                        .to_string();
                    assert!(error.contains("installation cancelled"), "{error}");
                    assert!(stage.is_dir() && prefix.is_dir());
                }
                unsafe {
                    let mut mask = std::mem::zeroed();
                    assert_eq!(
                        libc::pthread_sigmask(libc::SIG_SETMASK, std::ptr::null(), &mut mask),
                        0
                    );
                    for signal in INSTALL_SIGNALS {
                        assert_eq!(
                            libc::sigismember(&mask, signal),
                            libc::sigismember(&previous, signal)
                        );
                    }
                }
            }
        }

        #[test]
        fn cancellation_after_publication_keeps_the_committed_tree() {
            let fixture = Fixture(
                Path::new("/var/tmp").join(format!("frayd-native-commit-{}", uuid::Uuid::new_v4())),
            );
            private_dir(&fixture.0).unwrap();
            let stage = fixture.0.join("stage");
            let prefix = fixture.0.join("prefix");
            private_dir(&stage).unwrap();
            private_dir(&prefix).unwrap();
            fs::write(stage.join("published"), b"committed").unwrap();
            {
                let signals = InstallSignals::block().unwrap();
                publish_install(&stage, &prefix, &signals).unwrap();
                assert_eq!(
                    unsafe { libc::pthread_kill(libc::pthread_self(), libc::SIGQUIT) },
                    0
                );
            }
            assert!(!stage.exists());
            assert_eq!(fs::read(prefix.join("published")).unwrap(), b"committed");
        }

        #[test]
        fn rollback_refuses_replaced_file_and_directory_identities() {
            let fixture = Fixture(
                Path::new("/var/tmp")
                    .join(format!("frayd-native-replaced-{}", uuid::Uuid::new_v4())),
            );
            private_dir(&fixture.0).unwrap();
            for directory in [false, true] {
                let path = fixture.0.join(if directory { "directory" } else { "file" });
                if directory {
                    private_dir(&path).unwrap();
                } else {
                    fs::write(&path, b"owned").unwrap();
                }
                let owned = CreatedPath::record(path.clone()).unwrap();
                fs::rename(&path, path.with_extension("original")).unwrap();
                if directory {
                    private_dir(&path).unwrap();
                } else {
                    fs::write(&path, b"preserve replacement").unwrap();
                }
                assert!(owned
                    .remove()
                    .unwrap_err()
                    .to_string()
                    .contains("path changed"));
                assert!(path.exists() && path.with_extension("original").exists());
            }
        }

        #[test]
        fn failed_stage_metadata_preserves_stage_and_only_cleans_matching_empty_reservation() {
            let fixture = Fixture(Path::new("/var/tmp").join(format!(
                "frayd-native-stage-record-{}",
                uuid::Uuid::new_v4()
            )));
            private_dir(&fixture.0).unwrap();
            for state in ["empty", "nonempty", "replaced"] {
                let root = fixture.0.join(state);
                private_dir(&root).unwrap();
                let prefix = root.join("prefix");
                private_dir(&prefix).unwrap();
                let reservation = fs::symlink_metadata(&prefix).unwrap();
                let stage = root.join("stage");
                private_dir(&stage).unwrap();
                fs::write(stage.join("unqualified"), b"preserve").unwrap();
                if state == "nonempty" {
                    fs::write(prefix.join("foreign"), b"preserve").unwrap();
                } else if state == "replaced" {
                    fs::rename(&prefix, root.join("original")).unwrap();
                    private_dir(&prefix).unwrap();
                }
                let error = retain_unqualified_stage(
                    &prefix,
                    &reservation,
                    &stage,
                    io::Error::new(io::ErrorKind::PermissionDenied, "metadata fixture denied")
                        .into(),
                )
                .to_string();
                assert!(error.contains("metadata fixture denied"), "{error}");
                assert!(error.contains(&stage.display().to_string()), "{error}");
                assert_eq!(fs::read(stage.join("unqualified")).unwrap(), b"preserve");
                assert_eq!(prefix.exists(), state != "empty");
                if state != "empty" {
                    assert!(error.contains(&prefix.display().to_string()), "{error}");
                }
            }
        }

        #[test]
        fn cleanup_refuses_substituted_reservation() {
            let fixture = Fixture(
                Path::new("/var/tmp")
                    .join(format!("frayd-native-identity-{}", uuid::Uuid::new_v4())),
            );
            private_dir(&fixture.0).unwrap();
            let prefix = fixture.0.join("prefix");
            private_dir(&prefix).unwrap();
            let identity = fs::symlink_metadata(&prefix).unwrap();
            fs::rename(&prefix, fixture.0.join("original")).unwrap();
            private_dir(&prefix).unwrap();
            let error = remove_reservation(&prefix, &identity)
                .unwrap_err()
                .to_string();
            assert!(error.contains("reservation changed"), "{error}");
            assert!(prefix.is_dir());
            assert!(fixture.0.join("original").is_dir());
        }

        #[test]
        fn child_marker_update_never_follows_replacement_symlink() {
            let fixture = Fixture(
                Path::new("/var/tmp").join(format!("frayd-native-child-{}", uuid::Uuid::new_v4())),
            );
            private_dir(&fixture.0).unwrap();
            let path = fixture.0.join(CHILD);
            let sentinel = fixture.0.join("sentinel");
            fs::write(&sentinel, b"preserve").unwrap();
            let mut file = private_file(&path, b"unknown").unwrap();
            fs::rename(&path, fixture.0.join("original")).unwrap();
            symlink(&sentinel, &path).unwrap();
            record_child(&mut file, Some(42)).unwrap();
            assert_eq!(fs::read(&sentinel).unwrap(), b"preserve");
            let document: serde_json::Value =
                serde_json::from_slice(&fs::read(fixture.0.join("original")).unwrap()).unwrap();
            assert_eq!(document["pid"], 42);
            assert!(remove_private_file(&path, &file)
                .unwrap_err()
                .to_string()
                .contains("metadata changed"));
            assert!(fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink());
        }

        #[test]
        fn marked_data_is_reported() {
            scenario("marked", false);
        }
        #[test]
        fn marker_metadata_error_continues_cleanup() {
            scenario("uncertain", false);
        }
        #[test]
        fn prefix_metadata_error_retains_uncertain_path() {
            scenario("marked", true);
        }
        #[test]
        fn empty_unmarked_data_is_removed() {
            scenario("empty", false);
        }
        #[test]
        fn nonempty_unmarked_data_is_retained() {
            scenario("nonempty", false);
        }

        #[test]
        fn manifest_refuses_empty_absolute_reserved_and_redirecting_paths() {
            let file = |path: &str| Entry {
                path: path.into(),
                kind: Kind::File,
                mode: 0o644,
                size: 0,
                sha256: "0".repeat(64),
            };
            let dir = |path: &str| Entry {
                path: path.into(),
                kind: Kind::Dir,
                mode: 0o755,
                size: 0,
                sha256: String::new(),
            };
            let executable = |path: &str| Entry {
                mode: 0o755,
                ..file(path)
            };
            let complete = || {
                let mut entries: Vec<Entry> = [
                    "frayd-server",
                    "frayd-native",
                    "install.sh",
                    "start.sh",
                    "uninstall.sh",
                ]
                .into_iter()
                .map(executable)
                .collect();
                entries.extend([file("README.md"), dir("client"), file("client/index.html")]);
                Manifest {
                    schema: 1,
                    version: "1".into(),
                    source: "a".repeat(40),
                    target: "t".into(),
                    toolchain: "t".into(),
                    entries,
                }
            };
            assert!(entry_names(&complete()).is_ok());
            for path in [
                "",
                "/frayd-server",
                "client/../README.md",
                "client//index.html",
                "client/./index.html",
                "README.md/x",
                "client\\x",
            ] {
                let mut manifest = complete();
                manifest.entries.push(file(path));
                assert!(entry_names(&manifest).is_err(), "{path:?} accepted");
            }
            for reserved in ["manifest.json", RECEIPT, LOCK, CHILD, DATA_RECEIPT] {
                let mut manifest = complete();
                manifest.entries.push(file(reserved));
                assert_eq!(
                    entry_names(&manifest).unwrap_err().to_string(),
                    "unexpected bundle payload; configuration and data are not install-owned"
                );
            }
            let mut duplicate = complete();
            duplicate.entries.push(file("README.md"));
            assert!(entry_names(&duplicate).is_err());
            // Only the exact lowercase kinds deserialize; entry_names does not check kind.
            let mut document = serde_json::to_value(complete()).unwrap();
            assert!(
                serde_json::from_slice::<Manifest>(&serde_json::to_vec(&document).unwrap()).is_ok()
            );
            for kind in ["symlink", "File"] {
                document["entries"][0]["kind"] = kind.into();
                let parsed =
                    serde_json::from_slice::<Manifest>(&serde_json::to_vec(&document).unwrap());
                assert!(parsed.is_err(), "kind {kind:?} accepted");
            }
        }
    }
}

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    native::main()
}

/// The helper is Unix-only; elsewhere this stub keeps the auto-discovered binary building.
#[cfg(not(unix))]
fn main() {
    eprintln!("frayd-native supports Linux/macOS only; the existing source bootstrap starts frayd-server directly");
    std::process::exit(2);
}
