#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("The Win32 frontend is only available on Windows.");
}

#[cfg(windows)]
mod app;

#[cfg(windows)]
fn main() {
    app::run();
}
