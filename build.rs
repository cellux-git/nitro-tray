fn main() {
    // The manifest embedding is Windows-only, and so is the build dependency:
    // [target.'cfg(windows)'.build-dependencies] matches the HOST platform, so
    // on a native Linux host `embed_manifest` is not in the graph at all —
    // the block must be cfg'd out (host) as well as guarded on the TARGET
    // (CARGO_CFG_TARGET_OS), so `cargo check --target x86_64-unknown-linux-gnu`
    // from Windows and a native Linux build both never reference the crate.
    #[cfg(windows)]
    {
        let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
        if target_os == "windows" {
            println!("cargo:rerun-if-changed=res/app.manifest");
            embed_manifest::embed_manifest_file("res/app.manifest").expect("embed manifest failed");
        }
    }
}
