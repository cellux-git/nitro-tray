fn main() {
    println!("cargo:rerun-if-changed=res/app.manifest");
    embed_manifest::embed_manifest_file("res/app.manifest").expect("embed manifest failed");
}
