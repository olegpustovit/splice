fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    println!("cargo:rerun-if-changed=native");
    cc::Build::new()
        .files([
            "native/error.m",
            "native/events.m",
            "native/media.m",
            "native/composition.m",
            "native/image.m",
        ])
        .flag("-fobjc-arc")
        .flag("-fblocks")
        .warnings_into_errors(true)
        .compile("splice_apple");
    for framework in [
        "Accelerate",
        "AVFoundation",
        "CoreGraphics",
        "CoreMedia",
        "CoreText",
        "CoreVideo",
        "Foundation",
        "ImageIO",
        "Metal",
        "QuartzCore",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    println!("cargo:rustc-link-lib=dylib=objc");
}
