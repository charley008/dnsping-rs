fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/app.ico");
        winresource::WindowsResource::new()
            .set_icon("assets/app.ico")
            .set("ProductName", "DNSPing")
            .set(
                "FileDescription",
                "DNSPing - DNS and network latency explorer",
            )
            .set("LegalCopyright", "DNSPing contributors")
            .compile()
            .expect("编译 Windows 图标资源失败");
    }
}
