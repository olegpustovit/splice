use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=native");
    println!("cargo:rerun-if-changed=vendor");
    let mut sources = vec![PathBuf::from("native/shim.cpp")];
    let mut includes = vec![PathBuf::from("native"), PathBuf::from("vendor/inc")];
    walk(Path::new("vendor/src"), &mut sources, &mut includes);
    sources.sort();
    // cc compiles one file at a time without its `parallel` feature (which needs more crates),
    // so the build script splits the ~240 translation units across threads itself.
    let jobs = std::env::var("NUM_JOBS")
        .ok()
        .and_then(|jobs| jobs.parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, 32);
    let chunk = sources.len().div_ceil(jobs);
    let objects: Vec<PathBuf> = std::thread::scope(|scope| {
        let workers: Vec<_> = sources
            .chunks(chunk)
            .map(|files| {
                let includes = &includes;
                scope.spawn(move || {
                    let mut build = configure(includes);
                    build.files(files).compile_intermediates()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("compile thread"))
            .collect()
    });
    configure(&includes)
        .objects(objects)
        .compile("splice_thorvg");
}

/// The flags ThorVG's meson release build uses (C++17 here), without warnings.
fn configure(includes: &[PathBuf]) -> cc::Build {
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .includes(includes)
        .define("TVG_STATIC", None)
        .define("NDEBUG", None)
        .flag("-fno-exceptions")
        .flag("-fno-rtti")
        .flag("-fno-math-errno")
        .flag("-fvisibility=hidden")
        .flag("-w")
        .opt_level(2)
        .debug(false)
        .warnings(false);
    build
}

/// Collects every `.cpp` under `dir`, and every directory as an include path (RapidJSON's
/// headers are included by their `rapidjson/` path).
fn walk(dir: &Path, sources: &mut Vec<PathBuf>, includes: &mut Vec<PathBuf>) {
    if dir.file_name().is_some_and(|name| name == "rapidjson") {
        return;
    }
    includes.push(dir.to_path_buf());
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("vendor directory")
        .map(|entry| entry.expect("vendor entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            walk(&path, sources, includes);
        } else if path.extension().is_some_and(|ext| ext == "cpp") {
            sources.push(path);
        }
    }
}
