//! Compiles the Slint UI and bundles translations into the binary.
//!
//! Bundled translations (rather than runtime gettext) are used so that language
//! switching works on Android and iOS, where placing `.mo` files on disk is
//! impractical. See docs/ja/develop/design/ui/i18n-system.md.

fn main() {
    let translations = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../i18n");

    // The element-query API used by the interaction tests can only see the item
    // tree when the generated code carries debug info. Release builds omit it so
    // the shipped binary stays lean.
    let debug_info = std::env::var("DEBUG").is_ok_and(|value| value != "false" && value != "0");

    let config = slint_build::CompilerConfiguration::new()
        .with_bundled_translations(&translations)
        .with_debug_info(debug_info);

    slint_build::compile_with_config("ui/main.slint", config)
        .expect("failed to compile ui/main.slint");

    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-changed={}", translations.display());
}
