// Ícono y datos de versión del ejecutable (Explorador, barra de tareas, Propiedades).
fn main() {
    println!("cargo:rerun-if-changed=../../../installer/kbss.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../../installer/kbss.ico")
        .set("ProductName", "KB Studio Screen")
        .set("FileDescription", "KB Studio Screen")
        .set("CompanyName", "Castro Labs")
        .set("LegalCopyright", "(c) 2026 Castro Labs");
    if let Err(e) = res.compile() {
        println!("cargo:warning=no se pudo incrustar el ícono: {e}");
    }
}
