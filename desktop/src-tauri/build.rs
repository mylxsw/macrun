fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/platform.m")
            .flag("-std=c11")
            .compile("macrun_platform");
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=IOKit");
        println!("cargo:rerun-if-changed=src/platform.m");
    }
    tauri_build::build()
}
