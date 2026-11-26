fn main(){
    #[cfg(feature = "winfsp-vault")]
    winfsp::build::winfsp_link_delayload();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref()==Ok("msvc"){
        let manifest=std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("app.manifest");
        println!("cargo:rerun-if-changed=app.manifest");
        println!("cargo:rustc-link-arg-bin=ndr=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bin=ndr=/MANIFESTINPUT:{}",manifest.display());
    }
    // Embed the same multi-resolution icon used by the running window.
    // Keep the existing DPI/compatibility manifest independent of the icon.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/ndr.ico");
        println!("cargo:rerun-if-changed=assets/ndr-icon.png");
        println!("cargo:rerun-if-changed=assets/ndr.rc");
        let mut resource = winresource::WindowsResource::new();
        resource.set_resource_file("assets/ndr.rc");
        resource.compile().expect("compile NDR application icon");
    }
}
