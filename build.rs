//! Windows only: put the app icon and its details into the .exe, so Explorer
//! and the taskbar show them. Nothing to do on a Mac (the icon is in the .app).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/AppIcon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/AppIcon.ico");
        res.set("ProductName", "PDF Notes");
        res.set("FileDescription", "PDF Notes");
        if let Err(e) = res.compile() {
            // Not fatal: the program works, it just has the plain icon.
            println!("cargo:warning=could not add the icon to the .exe: {e}");
        }
    }
}
