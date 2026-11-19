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
        _ => "unknown".to_owned(),
    };
    println!("cargo:rustc-env=SPLICE_BUILD={build}");
    // Rerun when HEAD moves or the index changes; source edits in this crate rerun it anyway.
    let branch = git(&["rev-parse", "--symbolic-full-name", "HEAD"]).unwrap_or_default();
    for path in ["HEAD", "index", "packed-refs", branch.as_str()] {
        if let Some(file) = git(&["rev-parse", "--git-path", path]).filter(|_| !path.is_empty()) {
            println!("cargo:rerun-if-changed={file}");
        }
    }
    println!("cargo:rerun-if-changed=src");
}
