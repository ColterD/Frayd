use std::{env, process::Command};

fn git(args: &[&str]) -> Option<String> {
    let root = std::path::PathBuf::from(env::var("CARGO_MANIFEST_DIR").ok()?).parent()?.to_owned();
    if !root.join(".git").exists() { return None; }
    let output = Command::new("git").current_dir(root).args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    for key in ["FRAYD_BUILD_COMMIT", "FRAYD_BUILD_COMMITS_SINCE_TAG", "FRAYD_BUILD_DIRTY"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    let explicit = env::var("FRAYD_BUILD_COMMIT").ok();
    println!("cargo:rerun-if-changed=build.rs");
    if explicit.is_none() {
        // Watch files and bounded source directories, never target or private evidence.
        for relative in ["../client", "src", "../scripts", "../docs", "../.github"] {
            println!("cargo:rerun-if-changed={relative}");
        }
        if let Some(files) = git(&["ls-files", "--cached", "--others", "--exclude-standard"]) {
            for file in files.lines() { println!("cargo:rerun-if-changed=../{file}"); }
        }
        // Git worktrees have a .git file; resolve their actual metadata paths.
        for name in ["HEAD", "index", "packed-refs", "refs"] {
            if let Some(path) = git(&["rev-parse", "--path-format=absolute", "--git-path", name]) {
                println!("cargo:rerun-if-changed={path}");
            }
        }
    }
    let commit = explicit.clone().or_else(|| git(&["rev-parse", "HEAD"])).unwrap_or_default();
    let commit = if commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()) { commit.to_ascii_lowercase() } else { "unknown".into() };
    let count = env::var("FRAYD_BUILD_COMMITS_SINCE_TAG").ok().or_else(|| {
        if explicit.is_some() || git(&["rev-parse", "--is-shallow-repository"]).as_deref() != Some("false") { return None; }
        let tag = git(&["describe", "--tags", "--abbrev=0"])?;
        git(&["rev-list", "--count", &format!("{tag}..HEAD")])
    }).and_then(|v| v.parse::<u64>().ok()).map(|v|v.to_string()).unwrap_or_else(||"unknown".into());
    let dirty = env::var("FRAYD_BUILD_DIRTY").ok().or_else(|| {
        if explicit.is_some() { return None; }
        git(&["status", "--porcelain", "--untracked-files=normal"]).map(|v| (!v.is_empty()).to_string())
    }).filter(|v| matches!(v.as_str(), "true" | "false")).unwrap_or_else(||"unknown".into());
    println!("cargo:rustc-env=FRAYD_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=FRAYD_BUILD_COMMITS_SINCE_TAG={count}");
    println!("cargo:rustc-env=FRAYD_BUILD_DIRTY={dirty}");
}
