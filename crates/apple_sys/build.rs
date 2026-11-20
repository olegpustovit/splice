fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    println!("cargo:rerun-if-changed=native");
    cc::Build::new()
        .files([
            "native/analysis.m",
            "native/app.m",
            "native/audio.m",
            "native/capture.m",
            "native/caret.m",
            "native/cursor.m",
            "native/error.m",
            "native/events.m",
            "native/input_events.m",
            "native/media.m",
            "native/composition.m",
            "native/html.m",
            "native/image.m",
            "native/permissions.m",
            "native/recording.m",
            "native/recording_fixture.m",
        ])
        .flag("-fobjc-arc")
        .flag("-fblocks")
        .warnings_into_errors(true)
        .compile("splice_apple");
    swift();
    for framework in [
        "Accelerate",
        "AppKit",
        "ApplicationServices",
        "AudioToolbox",
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
        "SoundAnalysis",
        "Speech",
        "Translation",
        "Vision",
        "WebKit",
        "CoreImage",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    println!("cargo:rustc-link-lib=dylib=objc");
}

/// Swift-only Apple APIs (SpeechAnalyzer) export C symbols with `@_cdecl`; compile them into a
/// static library and link the Swift runtime, which macOS ships in /usr/lib/swift.
fn swift() {
    use std::process::Command;
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok(other) => other,
        Err(_) => "arm64",
    }
    .to_owned();
    let library = out.join("libsplice_swift.a");
    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-parse-as-library",
            "-emit-library",
            "-static",
            "-O",
        ])
        .args(["-module-name", "splice_swift", "-target"])
        .arg(format!("{arch}-apple-macos15.0"))
        .arg("-o")
        .arg(&library)
        .args(["native/speech.swift", "native/translate.swift"])
        .status()
        .expect("xcrun swiftc");
    assert!(status.success(), "swiftc failed for native/*.swift");
    let path = |args: &[&str]| {
        let output = Command::new("xcrun").args(args).output().expect("xcrun");
        String::from_utf8(output.stdout)
            .expect("utf-8 path")
            .trim()
            .to_owned()
    };
    let sdk = path(&["--show-sdk-path"]);
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=splice_swift");
    // Only the SDK's stubs: they name the OS runtime in /usr/lib/swift. The toolchain's
    // back-deployment copies would leave @rpath references that dyld cannot resolve.
    println!("cargo:rustc-link-search=native={sdk}/usr/lib/swift");
}
