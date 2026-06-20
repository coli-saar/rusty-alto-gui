fn main() {
    // On macOS, embed an Info.plist into the binary's __TEXT,__info_plist
    // section so that even a loose `cargo run` executable reports its
    // CFBundleName ("Rusty Alto") — which is what the menu bar shows as the
    // application name. Without this, the name is the executable filename.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        println!(
            "cargo:rustc-link-arg=-Wl,-sectcreate,__TEXT,__info_plist,{manifest}/Info.plist"
        );
        println!("cargo:rerun-if-changed=Info.plist");
    }
}
