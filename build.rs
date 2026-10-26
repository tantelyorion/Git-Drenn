//! Intègre l'icône de la mascotte (assets/drenn.ico) dans drenn.exe quand la fonctionnalité `icon` est activée.
fn main() {
    println!("cargo:rerun-if-changed=assets/drenn.ico");
    #[cfg(feature = "icon")]
    {
        if std::env::var("CARGO_CFG_TARGET_OS").map(|o| o == "windows").unwrap_or(false) {
            let mut res = winres::WindowsResource::new();
            res.set_icon("assets/drenn.ico");
            res.set("ProductName", "GIT DRENN");
            res.set("FileDescription", "GIT DRENN - Git Powered by AI");
            if let Err(e) = res.compile() {
                println!("cargo:warning=icône non intégrée : {e}");
            }
        }
    }
}
