//! Compiles translations and embeds Windows resources.

fn main() {
    fastframe_i18n::build::compile_catalogs("assets/i18n");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=packaging/windows/melba.ico");
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("packaging/windows/melba.ico")
            .set("ProductName", "melba")
            // Task Manager and the Windows mixer name the process by its description.
            .set("FileDescription", "melba");
        if let Err(error) = resource.compile() {
            println!("cargo:warning=Windows resources not embedded: {error}");
        }
    }
}
