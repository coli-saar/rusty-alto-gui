fn main() {
    // On macOS, embed an Info.plist into the binary's __TEXT,__info_plist
    // section so that even a loose `cargo run` executable reports its
    // CFBundleName ("Rusty Alto") — which is what the menu bar shows as the
    // application name. Without this, the name is the executable filename.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let plist = std::path::Path::new(&manifest).join("Info.plist");
        println!(
            "cargo:rustc-link-arg=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            plist.display()
        );
        // Explicitly tracking the build script invalidates stale linker arguments
        // when a checkout is moved while retaining its target directory.
        println!("cargo:rerun-if-changed=build.rs");
        println!("cargo:rerun-if-changed=Info.plist");
    }
}
