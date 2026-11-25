//! Embeds the icon, the application manifest (Per-Monitor V2 DPI, common
//! controls v6, asInvoker, long paths) and the version information.

fn main() {
    println!("cargo:rerun-if-changed=res/app.rc");
    println!("cargo:rerun-if-changed=res/app.manifest");
    println!("cargo:rerun-if-changed=../../assets/bento.ico");
    if let Err(e) = embed_resource::compile("res/app.rc", embed_resource::NONE).manifest_required()
    {
        panic!("resource compilation failed: {e}");
    }
}
