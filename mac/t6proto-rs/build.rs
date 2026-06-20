fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rerun-if-changed=src/macos_virtual_display.m");
        println!("cargo:rerun-if-changed=src/macos_vt_jpeg_probe.m");

        cc::Build::new()
            .file("src/macos_virtual_display.m")
            .file("src/macos_vt_jpeg_probe.m")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .flag("-Wno-nullability-completeness")
            .flag("-Wno-deprecated-declarations")
            .compile("macos_virtual_display");

        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
        println!("cargo:rustc-link-lib=framework=CoreMedia");
        println!("cargo:rustc-link-lib=framework=CoreVideo");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=IOSurface");
        println!("cargo:rustc-link-lib=framework=VideoToolbox");
        println!("cargo:rustc-link-lib=framework=Accelerate");
    }
}
