fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/anmedia.ico");
        println!("cargo:rerun-if-changed=assets/app.manifest");
        let mut resource = winres::WindowsResource::new();
        resource.set_icon("assets/anmedia.ico");
        resource.set_manifest_file("assets/app.manifest");
        resource
            .compile()
            .expect("failed to compile Windows resources");
    }
}
