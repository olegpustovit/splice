//! Build identity (T36.9): `SPLICE_BUILD` is `<git short sha>[-dirty]`, or `unknown` outside a
//! git checkout, so `serverInfo.version` says exactly which build an agent talks to.
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    let build = match git(&["rev-parse", "--short=9", "HEAD"]) {
        Some(sha) if !sha.is_empty() => {
            let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
                .is_some_and(|status| !status.is_empty());
            if dirty { format!("{sha}-dirty") } else { sha }
        }
        // A tree synced without .git (the Mac): `make mac-sync` writes the identity next to it.
        _ => std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.build-sha"))
            .ok()
            .map(|sha| sha.trim().to_owned())
            .filter(|sha| !sha.is_empty())
            .unwrap_or_else(|| "unknown".to_owned()),
    };
    println!("cargo:rustc-env=SPLICE_BUILD={build}");
    // Rerun when HEAD moves (the SHA is exact). `-dirty` describes the tree when this crate last
    // built: watching the index would rebuild splice_mcp and everything above it on every
    // `git add`, which costs more than a stale flag on a dev build.
    let branch = git(&["rev-parse", "--symbolic-full-name", "HEAD"]).unwrap_or_default();
    for path in ["HEAD", "packed-refs", branch.as_str()] {
        if let Some(file) = git(&["rev-parse", "--git-path", path]).filter(|_| !path.is_empty()) {
            println!("cargo:rerun-if-changed={file}");
        }
    }
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../../.build-sha");
}
