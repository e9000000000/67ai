fn main() {
    println!("cargo:rerun-if-changed=assets/icons/67ai.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icons/67ai.ico")
            .compile()
            .expect("Could not embed the 67ai icon");
    }
}
