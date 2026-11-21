fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent-dark".into());
    slint_build::compile_with_config("ui/main.slint", config).expect("compile ui/main.slint");
    // Slint recommends a larger main-thread stack on Windows (debug builds overflow the 1 MB default).
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/STACK:8000000");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("ui/icons/app.ico");
        resource.set("ProductName", "Senvakara");
        resource.set("FileDescription", "Senvakara");
        resource.compile().expect("embed Windows resources");
    }
}
