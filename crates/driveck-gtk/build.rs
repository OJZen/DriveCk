#[cfg(target_os = "linux")]
fn main() {
    glib_build_tools::compile_resources(
        &["../../icon"],
        "resources/driveck.gresource.xml",
        "driveck.gresource",
    );
}

#[cfg(not(target_os = "linux"))]
fn main() {}
