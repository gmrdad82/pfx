fn main() {
    println!("cargo:rerun-if-changed=../../assets/marks/pfx.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let icon = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/marks/pfx.ico");
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon(icon)
        .set("ProductName", "pfx")
        .set("FileDescription", "pfx")
        .set("InternalName", "pfx")
        .set("OriginalFilename", "pfx.exe");
    if let Err(error) = resource.compile() {
        panic!("pfx.exe's icon resource from {icon}: {error}");
    }
}
