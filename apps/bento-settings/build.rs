//! Compiles the Slint interface (Fluent style, French translations bundled
//! in the binary) and embeds the icon, the manifest and the version
//! information.

fn main() {
    println!("cargo:rerun-if-changed=res/settings.rc");
    println!("cargo:rerun-if-changed=res/app.manifest");
    println!("cargo:rerun-if-changed=lang");
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_bundled_translations("lang")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    if let Err(e) = slint_build::compile_with_config("ui/app.slint", config) {
        panic!("Slint compilation failed: {e}");
    }
    if let Err(e) =
        embed_resource::compile("res/settings.rc", embed_resource::NONE).manifest_required()
    {
        panic!("resource compilation failed: {e}");
    }
}
