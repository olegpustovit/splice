//! Project policy checks. Run with `cargo run -p gate -- all`.
mod lex;

use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args().nth(1);
    if !matches!(mode.as_deref(), None | Some("all")) {
        return Err("usage: gate all".into());
    }
    let mut violations = Vec::new();
    for entry in fs::read_dir("crates")? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_str().ok_or("crate name must be UTF-8")?;
        let path = entry.path();
        check_sources(&path, name.ends_with("_sys"), &mut violations)?;
        let manifest = fs::read_to_string(path.join("Cargo.toml"))?;
        for rule in manifest_violations(name, &manifest) {
            violations.push(format!("{}: {rule}", path.join("Cargo.toml").display()));
        }
        if is_mac(name) {
            let source = fs::read_to_string(path.join("src/lib.rs"))?;
            if !mac_gated(&source) {
                violations.push(format!("{name}: missing crate-level macOS cfg"));
            }
        }
    }
    violations.sort();
    for violation in &violations {
        eprintln!("{violation}");
    }
    if !violations.is_empty() {
        return Err("Splice policy gate failed".into());
    }
    Ok(())
}

fn is_mac(name: &str) -> bool {
    matches!(
        name,
        "apple_sys" | "splice_media" | "splice_av" | "splice_capture" | "splice_app"
    )
}

fn mac_gated(source: &str) -> bool {
    source
        .lines()
        .any(|line| line.split_whitespace().collect::<String>() == "#![cfg(target_os=\"macos\")]")
}

fn check_sources(dir: &Path, sys: bool, violations: &mut Vec<String>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            check_sources(&path, sys, violations)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let source = fs::read_to_string(&path)?;
            for (line, rule) in source_issues(&source, sys) {
                violations.push(format!("{}:{line}: {rule}", path.display()));
            }
        }
    }
    Ok(())
}

fn source_issues(source: &str, sys: bool) -> Vec<(usize, &'static str)> {
    let tokens = lex::lex(source);
    let words: Vec<&str> = tokens.iter().map(|token| token.text.as_str()).collect();
    let mut issues = Vec::new();
    for (index, &word) in words.iter().enumerate() {
        let next = words.get(index + 1).copied().unwrap_or("");
        let after = words.get(index + 2).copied().unwrap_or("");
        let attr = index >= 1 && words[index - 1] == "[";
        let rule = match word {
            "unsafe" if !sys => "unsafe",
            "async" => "async",
            "macro_rules" if next == "!" => "macro_rules",
            "proc_macro" | "proc_macro_attribute" | "proc_macro_derive" if attr => "proc macro",
            "ignore" if attr => "ignored test",
            // ponytail: reserve HashMap for a measured use that needs unordered storage.
            // Until then ordered collections also make output stable by construction.
            "HashMap" => "HashMap",
            "fn" | "struct" | "enum" | "union" | "type" | "trait" if after == "<" => "generic item",
            "impl" if next == "<" => "generic item",
            "impl"
                if matches!(
                    words.get(index.wrapping_sub(1)),
                    Some(&":" | &"->" | &"=" | &"<" | &"," | &"(")
                ) =>
            {
                "impl Trait"
            }
            "derive" if attr && next == "(" => {
                let allowed = [
                    "Clone",
                    "Copy",
                    "Debug",
                    "PartialEq",
                    "Eq",
                    "Hash",
                    "Default",
                    ",",
                ];
                if words[index + 2..]
                    .iter()
                    .take_while(|&&word| word != ")")
                    .all(|word| allowed.contains(word))
                {
                    continue;
                }
                "derive"
            }
            _ => continue,
        };
        issues.push((tokens[index].line, rule));
    }
    issues
}

// ponytail: support the workspace's one-entry-per-line Cargo manifest style.
// Use Cargo metadata when dependency aliases or multiline dependency tables are needed.
fn manifest_violations(name: &str, manifest: &str) -> Vec<&'static str> {
    let allowed: &[&str] = match name {
        "splice_cli" => &["splice_app", "splice_core", "splice_json", "splice_mcp"],
        "splice_app" => &[
            "apple_sys",
            "input",
            "metal_sys",
            "ogpu_core",
            "ogpu_metal",
            "pxui",
            "pxui_host",
            "pxui_sdl",
            "splice_av",
            "splice_core",
            "splice_gpu",
            "splice_json",
            "splice_kit",
            "splice_mcp",
            "splice_model",
        ],
        "splice_av" => &[
            "apple_sys",
            "ogpu_core",
            "splice_eval",
            "splice_gpu",
            "splice_json",
            "splice_model",
        ],
        // D1: the portable compositor renders through PXUI's open-tier OGPU.
        "splice_gpu" => &["ogpu_core"],
        "splice_capture" => &["apple_sys", "splice_json"],
        "splice_core" => &["splice_model", "splice_json"],
        "splice_eval" => &["splice_model"],
        // D2: QuickJS-NG for code mode; size and build time in crates/quickjs_sys/README.md.
        "splice_mcp" => &[
            "apple_sys",
            "quickjs_sys",
            "splice_av",
            "splice_capture",
            "splice_core",
            "splice_eval",
            "splice_json",
            "splice_media",
            "splice_model",
        ],
        "splice_media" => &["apple_sys", "splice_model"],
        "splice_testkit" => &["splice_core", "splice_json"],
        "splice_kit" => &["pxui_types"],
        _ => &[],
    };
    let mut issues = Vec::new();
    let mut table = "";
    for line in manifest.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            table = line.trim_matches(['[', ']']);
            if table.contains("-dependencies.") || table.contains("dependencies.") {
                issues.push("use an inline dependency entry");
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            if table.ends_with("dependencies") {
                issues.push("dependency entry must be on one line");
            }
            continue;
        };
        let key = key.trim();
        if key == "proc-macro" && value.trim().starts_with("true") {
            issues.push("proc macro");
        }
        if !table.ends_with("dependencies") {
            continue;
        }
        let allowed_test = (matches!(key, "mem_sys" | "splice_testkit")
            || (name == "splice_eval" && key == "splice_core"))
            && table.ends_with("dev-dependencies");
        let allowed_build = key == "cc"
            && matches!(name, "apple_sys" | "quickjs_sys")
            && table.ends_with("build-dependencies");
        if !(allowed.contains(&key) || allowed_test || allowed_build) {
            issues.push("dependency is outside this crate's allowlist");
        }
        if value.contains("package") || value.contains("crates/private") {
            issues.push("dependency alias or private PXUI path");
        }
    }
    issues
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_prohibited_source_shapes() {
        for (source, rule) in [
            ("unsafe fn f() {}", "unsafe"),
            ("async fn f() {}", "async"),
            ("macro_rules! m { () => {} }", "macro_rules"),
            ("fn f(value: impl Copy) {}", "impl Trait"),
            ("fn f<T>() {}", "generic item"),
            ("struct S<T>(T);", "generic item"),
            ("#[proc_macro] fn f() {}", "proc macro"),
            ("#[derive(Clone, Ord)] struct S;", "derive"),
            ("#[ignore = \"slow\"] #[test] fn f() {}", "ignored test"),
            ("use std::collections::HashMap;", "HashMap"),
        ] {
            assert!(
                source_issues(source, false)
                    .iter()
                    .any(|issue| issue.1 == rule),
                "{source}"
            );
        }
    }

    #[test]
    fn enforces_dependencies_and_macos_gating() {
        assert!(
            manifest_violations("splice_core", "[dependencies]\nserde = \"1\"")
                .contains(&"dependency is outside this crate's allowlist")
        );
        assert!(
            manifest_violations(
                "splice_kit",
                "[dependencies]\nsplice_core = { path = \"../splice_core\" }"
            )
            .contains(&"dependency is outside this crate's allowlist")
        );
        assert!(
            manifest_violations(
                "splice_kit",
                "[dependencies]\npxui_types = { path = \"../../pxui/crates/private/pxui_types\" }"
            )
            .contains(&"dependency alias or private PXUI path")
        );
        assert!(
            manifest_violations("splice_model", "[lib]\nproc-macro = true").contains(&"proc macro")
        );
        assert!(
            manifest_violations(
                "splice_model",
                "[dev-dependencies]\nmem_sys = { path = \"../../pxui/crates/open/mem_sys\" }"
            )
            .is_empty()
        );
        assert!(manifest_violations("apple_sys", "[build-dependencies]\ncc = \"1\"").is_empty());
        assert!(manifest_violations("quickjs_sys", "[build-dependencies]\ncc = \"1\"").is_empty());
        assert!(!manifest_violations("splice_core", "[build-dependencies]\ncc = \"1\"").is_empty());
        assert!(
            manifest_violations(
                "splice_mcp",
                "[dependencies]\nquickjs_sys = { path = \"../quickjs_sys\" }"
            )
            .is_empty()
        );
        assert!(is_mac("splice_capture"));
        assert!(!mac_gated("// #![cfg(target_os = \"macos\")]"));
        assert!(mac_gated("#![cfg(target_os = \"macos\")]\nmod capture;"));
    }

    #[test]
    fn allows_comments_literals_and_safe_sys_wrapper() {
        assert!(
            source_issues(
                r##"//! unsafe async
            /* #[ignore] /* nested */ */
            const S: &str = r#"async unsafe macro_rules!"#;
            const T: &str = "unsafe \" async";
            #[derive(Clone, Copy)] struct S;
            unsafe extern "C" { fn probe(); }"##,
                true,
            )
            .is_empty()
        );
    }
}
