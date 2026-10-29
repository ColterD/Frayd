//! Native lifecycle authority: no guessed ownership, recursive deletion, or service control.
#[cfg(unix)]
mod native {
    use serde::{Deserialize, Serialize};
    use sha2::{Digest, Sha256};
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs::{self, DirBuilder, File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    use std::os::unix::process::ExitStatusExt;
    use std::path::{Component, Path, PathBuf};
    use std::process::{Command, ExitCode};
    use walkdir::WalkDir;

    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
    const RECEIPT: &str = ".frayd-install.json";
    const DATA_RECEIPT: &str = ".frayd-data.json";
    const LOCK: &str = ".frayd-lock";
    const CHILD: &str = ".frayd-child.json";

    /// Serialized fingerprint of one relative payload path; unknown fields are refused.
    #[derive(Clone, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct Entry {
        path: String,
        kind: String,
        mode: u32,
        size: u64,
        sha256: String,
    }
    /// Untrusted package identity and payload inventory, validated before creation.
    #[derive(Clone, Deserialize, Serialize)]
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
    /// Return a refusal without filesystem or lifecycle effects.
    /// Callers supply a nonsecret diagnostic; the CLI maps this failure to exit 2.
    fn reject<T>(message: &str) -> Result<T> {
        Err(message.into())
    }
    /// Reserve a previously absent directory with owner-only requested permissions.
    /// The caller must validate the path and obtain confirmation first; an existing
    /// path or filesystem error fails without adopting that path.
    fn private_dir(path: &Path) -> Result<()> {
        DirBuilder::new().mode(0o700).create(path)?;
        Ok(())
    }
    /// Create a new private metadata file from caller-serialized receipt bytes.
    /// Existing paths are never replaced. A write/sync failure removes only the
    /// same newly opened inode when possible, otherwise reports retained metadata.
    fn private_file(path: &Path, bytes: &[u8]) -> Result<()> {
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
    /// Require an existing 0700 directory owned by the invoking Unix user.
    /// Ownership is read from the process UID on Linux or a read-only macOS UID
    /// probe. Passing this check alone does not establish payload ownership.
    fn private_owned_dir(path: &Path) -> Result<()> {
        let meta = fs::symlink_metadata(path)?;
        // Compare with a file created by this process without requiring a libc dependency.
        let current_uid = fs::metadata("/proc/self")
            .map(|m| m.uid())
            .unwrap_or_else(|_| {
                // On macOS, the executable's owner is not necessarily the invoking user.
                // `id` is a platform utility and emits no credentials.
                Command::new("/usr/bin/id")
                    .arg("-u")
                    .output()
                    .ok()
                    .and_then(|o| String::from_utf8(o.stdout).ok())
                    .and_then(|s| s.trim().parse().ok())
                    .unwrap_or(u32::MAX)
            });
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
            kind: if meta.is_file() { "file" } else { "dir" }.into(),
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
            ]
            .contains(&top)
                || (top != "client" && item.path.contains('/'))
            {
                return reject(
                    "unexpected bundle payload; configuration and data are not install-owned",
                );
            }
            if item.path.is_empty()
                || path.is_absolute()
                || path
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
                || item.path.contains('\\')
                || path.components().count() != item.path.split('/').count()
                || !["file", "dir"].contains(&item.kind.as_str())
                || item.mode > 0o777
                || !names.insert(item.path.clone())
            {
                return reject("invalid, duplicate or redirecting manifest path/type/mode");
            }
            if ["manifest.json", RECEIPT, LOCK, CHILD, DATA_RECEIPT].contains(&item.path.as_str()) {
                return reject("reserved manifest path");
            }
            if item.kind == "dir"
                && (item.size != 0 || !item.sha256.is_empty() || item.mode & 0o700 != 0o700)
            {
                return reject("invalid directory entry");
            }
            if item.kind == "file"
                && (item.sha256.len() != 64 || !item.sha256.bytes().all(|c| c.is_ascii_hexdigit()))
            {
                return reject("invalid checksum");
            }
        }
        for (required, kind, executable) in [
            ("frayd-server", "file", true),
            ("frayd-native", "file", true),
            ("install.sh", "file", true),
            ("start.sh", "file", true),
            ("uninstall.sh", "file", true),
            ("README.md", "file", false),
            ("client", "dir", false),
            ("client/index.html", "file", false),
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
        if manifest_file.kind != "file" {
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
        if record.kind != "file" || record.mode != 0o600 {
            return reject("invalid install receipt");
        }
        let receipt: Receipt = serde_json::from_slice(&fs::read(prefix.join(RECEIPT))?)?;
        if receipt.schema != 1
            || receipt.prefix != prefix
            || !disjoint(prefix, &receipt.data)
            || receipt.manifest_file.path != "manifest.json"
            || receipt.manifest_file.kind != "file"
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
        if lock.kind != "file" || lock.mode != 0o600 || lock.size != 0 {
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
        let data_record = if data.try_exists()? {
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
        private_dir(prefix)?;
        let reservation = fs::symlink_metadata(prefix)?;
        let stage = prefix
            .parent()
            .ok_or("missing parent")?
            .join(format!(".frayd-stage-{}", uuid::Uuid::new_v4()));
        if let Err(error) = private_dir(&stage) {
            fs::remove_dir(prefix)?;
            return Err(error);
        }
        let mut created_data = false;
        let mut created_files = Vec::new();
        let mut created_dirs = Vec::new();
        let result = (|| -> Result<()> {
            let mut dirs: Vec<_> = manifest
                .entries
                .iter()
                .filter(|e| e.kind == "dir")
                .collect();
            dirs.sort_by_key(|e| e.path.matches('/').count());
            for item in dirs {
                private_dir(&stage.join(&item.path))?;
                created_dirs.push(stage.join(&item.path));
            }
            for item in manifest
                .entries
                .iter()
                .filter(|e| e.kind == "file")
                .chain(std::iter::once(&manifest_file))
            {
                let mut source = File::open(bundle.join(&item.path))?;
                let mut dest = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(stage.join(&item.path))?;
                created_files.push(stage.join(&item.path));
                io::copy(&mut source, &mut dest)?;
                fs::set_permissions(
                    stage.join(&item.path),
                    fs::Permissions::from_mode(item.mode),
                )?;
            }
            // Directory modes are restored only after all private staging writes finish.
            for item in manifest.entries.iter().filter(|e| e.kind == "dir") {
                fs::set_permissions(
                    stage.join(&item.path),
                    fs::Permissions::from_mode(item.mode),
                )?;
            }
            verify_tree(&stage, &manifest, &["manifest.json"], false)?;
            verify_entry(&stage, &manifest_file, false)?;
            if !data.try_exists()? {
                private_dir(data)?;
                created_data = true;
                private_file(
                    &data.join(DATA_RECEIPT),
                    &serde_json::to_vec_pretty(&data_record)?,
                )?;
            }
            private_file(&stage.join(LOCK), b"")?;
            created_files.push(stage.join(LOCK));
            let receipt = Receipt {
                schema: 1,
                prefix: prefix.into(),
                data: data.into(),
                data_id: data_record.id,
                manifest,
                manifest_file,
            };
            private_file(&stage.join(RECEIPT), &serde_json::to_vec_pretty(&receipt)?)?;
            created_files.push(stage.join(RECEIPT));
            let current = fs::symlink_metadata(prefix)?;
            if !current.is_dir()
                || current.dev() != reservation.dev()
                || current.ino() != reservation.ino()
                || fs::read_dir(prefix)?.next().is_some()
            {
                return reject("prefix reservation changed; refusing publication");
            }
            fs::rename(&stage, prefix)?;
            Ok(())
        })();
        if let Err(error) = result {
            // Roll back only paths created in this attempt; never recurse through an
            // unexpected tree. Failure retains exact paths and reports the original error.
            let mut retained = Vec::new();
            if created_data
                && !data.join(DATA_RECEIPT).try_exists()?
                && fs::remove_dir(data).is_err()
            {
                retained.push(data.display().to_string());
            }
            for file in created_files.iter().rev() {
                if fs::remove_file(file).is_err() {
                    retained.push(file.display().to_string());
                }
            }
            for directory in created_dirs.iter().rev() {
                if fs::remove_dir(directory).is_err() {
                    retained.push(directory.display().to_string());
                }
            }
            if fs::remove_dir(&stage).is_err() {
                retained.push(stage.display().to_string());
            }
            let current = fs::symlink_metadata(prefix)?;
            if current.is_dir()
                && current.dev() == reservation.dev()
                && current.ino() == reservation.ino()
            {
                if fs::remove_dir(prefix).is_err() {
                    retained.push(prefix.display().to_string());
                }
            } else {
                retained.push(prefix.display().to_string());
            }
            return Err(format!(
                "install failed: {error}; retained paths: {retained:?}; data is never deleted"
            )
            .into());
        }
        println!("Installed owned prefix; data is preserved independently.");
        Ok(())
    }
    /// Launch the unchanged owned server with explicit private runtime paths.
    /// Validate prefix/data and hold the lifecycle lock before spawning. Normal
    /// SIGINT/SIGTERM terminates and reaps only the owned Child handle; a crash or
    /// SIGKILL leaves its uncertainty marker to block later start/removal safely.
    fn start(prefix: &Path, port: u16) -> Result<()> {
        let receipt = load_prefix(prefix, false)?;
        validate_start_data(&receipt)?;
        let _lock = locked(prefix)?;
        // Revalidate under the lifecycle lock, including orphan markers.
        validate_start_data(&load_prefix(prefix, false)?)?;
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
            // Register before child creation. Normal terminal INT/TERM reaps only this
            // Child handle. SIGKILL/crash still leaves the fail-closed orphan marker.
            let mut interrupt =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            private_file(&prefix.join(CHILD), b"{\"state\":\"spawning-or-unknown\"}")?;
            let child = tokio::process::Command::new(prefix.join("frayd-server"))
                .current_dir(&receipt.data)
                .env("FRAYD_DB", receipt.data.join("frayd.db"))
                .env("FRAYD_THUMBS", receipt.data.join("thumbs"))
                .env("FRAYD_TRANSCODES", receipt.data.join("transcodes"))
                .env("FRAYD_CLIENT_DIR", prefix.join("client"))
                .env("FRAYD_BIND", "127.0.0.1")
                .env("FRAYD_PORT", port.to_string())
                .spawn();
            let mut child = match child {
                Ok(child) => child,
                Err(error) => {
                    fs::remove_file(prefix.join(CHILD))?;
                    return Err(error.into());
                }
            };
            fs::write(
                prefix.join(CHILD),
                serde_json::to_vec(&serde_json::json!({"pid": child.id(), "state": "running"}))?,
            )?;
            let (status, interrupted) = tokio::select! {
                // Foreground signals can make both the signal and child wait ready.
                // Prefer the explicit interruption; a child-only INT/TERM is also
                // normal interruption, even if its wait completes first.
                biased;
                _ = interrupt.recv() => { child.start_kill()?; (child.wait().await?, true) },
                _ = terminate.recv() => { child.start_kill()?; (child.wait().await?, true) },
                result = child.wait() => {
                    let status = result?;
                    let interrupted = matches!(status.signal(), Some(2) | Some(15));
                    (status, interrupted)
                },
            };
            fs::remove_file(prefix.join(CHILD))?;
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
        let receipt = load_prefix(prefix, true)?;
        removal_parent_writable(prefix)?;
        if !confirm(
            yes,
            "Remove only unchanged owned installation files; keep all data",
        )? {
            return Ok(());
        }
        let _lock = locked(prefix)?;
        load_prefix(prefix, true)?;
        for item in receipt
            .manifest
            .entries
            .iter()
            .filter(|e| e.kind == "file")
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
            .filter(|e| e.kind == "dir")
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
            let current = fs::symlink_metadata(prefix)?;
            if current.is_dir()
                && current.dev() == identity.dev()
                && current.ino() == identity.ino()
            {
                for (name, bytes) in [(LOCK, b"".as_slice()), (RECEIPT, receipt_bytes.as_slice())] {
                    if !prefix.join(name).try_exists()? {
                        private_file(&prefix.join(name), bytes).map_err(|restore| format!("final removal failed: {error}; metadata recovery failed at {}: {restore}; retained prefix requires inspection", prefix.display()))?;
                    }
                }
            }
            return Err(format!("final removal failed: {error}; retained prefix {} and recovery metadata require retry/inspection", prefix.display()).into());
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
        let command = args.next().ok_or("expected install, start or uninstall")?;
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
    /// Expose the Unix CLI outcome as success or a nonsecret exit-2 diagnostic.
    /// Propagate declined operations as success and never turn an operation failure
    /// into a completed install, removal or process-management claim.
    pub fn main() -> ExitCode {
        match run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("frayd-native: {error}");
                ExitCode::from(2)
            }
        }
    }
}

#[cfg(unix)]
/// Enter the Unix lifecycle implementation on supported native platforms.
/// Platform-specific ownership and signal code stays behind the Unix cfg gate.
fn main() -> std::process::ExitCode {
    native::main()
}

#[cfg(not(unix))]
/// Refuse native-helper execution on unsupported platforms with exit 2.
/// Keep the auto-discovered binary compilable there so existing source bootstrap
/// builds can continue to launch frayd-server directly; no lifecycle runs here.
fn main() {
    eprintln!("frayd-native supports Linux/macOS only; the existing source bootstrap starts frayd-server directly");
    std::process::exit(2);
}
