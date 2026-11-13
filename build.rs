//! Embeds the app icon in grids.exe (Windows targets only).
fn main() {
    println!("cargo:rerun-if-changed=assets/icon/grids.rc");
    println!("cargo:rerun-if-changed=assets/icon/grids.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/icon/grids.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("grids.rc: resource compile failed");
    }
}
