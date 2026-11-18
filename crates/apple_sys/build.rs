fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    println!("cargo:rerun-if-changed=native");
    cc::Build::new()
        .files([
            "native/app.m",
            "native/capture.m",
            "native/error.m",
            "native/events.m",
            "native/input_events.m",
            "native/media.m",
            "native/composition.m",
            "native/image.m",
            "native/permissions.m",
            "native/recording.m",
            "native/recording_fixture.m",
        ])
        .flag("-fobjc-arc")
        .flag("-fblocks")
        .warnings_into_errors(true)
        .compile("splice_apple");
    for framework in [
        "Accelerate",
        "AppKit",
        "AVFoundation",
        "Carbon",
        "CoreGraphics",
        "CoreMedia",
        "CoreText",
        "CoreVideo",
        "Foundation",
        "ImageIO",
        "Metal",
        "QuartzCore",
        "ScreenCaptureKit",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    println!("cargo:rustc-link-lib=dylib=objc");
}
