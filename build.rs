//! Embeds the app icon and version information in the Windows executable.
//! Other targets need neither, so for them this does nothing.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=packaging/windows/downmark.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // Icon resource 1 is also what GPUI loads as the window icon. File and
    // product versions come from Cargo.toml.
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("packaging/windows/downmark.ico")
        .set("ProductName", "Downmark")
        .set("FileDescription", "Downmark")
        .set("InternalName", "downmark")
        .set("OriginalFilename", "Downmark.exe")
        .set("CompanyName", "Vidhan")
        .set(
            "LegalCopyright",
            "Copyright 2026 Vidhan. Apache License 2.0.",
        );
    if let Err(err) = resource.compile() {
        panic!("could not embed the Windows icon and version information: {err}");
    }
}
