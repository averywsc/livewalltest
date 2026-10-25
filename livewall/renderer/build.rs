fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").map(|v| v == "windows").unwrap_or(false) {
        let mut res = winres::WindowsResource::new();
        res.set_icon("../assets/app.ico");
        res.set_manifest(include_str!("app.manifest"));
        res.set("ProductName", "Live Wallpaper");
        res.set("FileDescription", "Live Wallpaper renderer");
        res.compile().expect("could not compile Windows resources (install the Windows SDK)");
    }
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=../assets/app.ico");
}
