// Embeds the application icon and version information (needs rc.exe from
// the Windows SDK). A missing resource compiler only costs the icon, so it
// is reported as a build warning rather than failing the build.
fn main() {
    println!("cargo:rerun-if-changed=../../assets/twm.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/twm.ico")
        .set("ProductName", "TilingWindowsManager")
        .set("FileDescription", "TilingWindowsManager")
        .set("LegalCopyright", "MIT License");
    if let Err(e) = res.compile() {
        println!("cargo:warning=icon/version resource not embedded: {e}");
    }
}
