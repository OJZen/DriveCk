#[cfg(windows)]
fn main() {
    let mut resource = winres::WindowsResource::new();
    resource.set_manifest_file("app.manifest");
    resource.compile().expect("compile Win32 resources");
}

#[cfg(not(windows))]
fn main() {}
