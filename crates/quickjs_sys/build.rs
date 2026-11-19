fn main() {
    println!("cargo:rerun-if-changed=native");
    println!("cargo:rerun-if-changed=vendor");
    let mut build = cc::Build::new();
    build
        .files([
            "vendor/dtoa.c",
            "vendor/libregexp.c",
            "vendor/libunicode.c",
            "vendor/quickjs.c",
            "native/shim.c",
        ])
        .std("gnu11")
        .define("_GNU_SOURCE", None)
        .define("QUICKJS_NG_BUILD", None)
        .opt_level(2)
        .debug(false)
        .warnings(false);
    // QuickJS's internal asserts stay on in debug and test builds.
    if std::env::var("PROFILE").as_deref() == Ok("release") {
        build.define("NDEBUG", None);
    }
    build.compile("splice_quickjs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        println!("cargo:rustc-link-lib=m");
    }
}
