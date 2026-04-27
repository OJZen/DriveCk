#![cfg_attr(windows, windows_subsystem = "windows")]

use std::process::ExitCode;

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("The Win32 frontend is only available on Windows.");
    ExitCode::from(1)
}

#[cfg(windows)]
mod app;

#[cfg(windows)]
use windows::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE},
    System::Console::{
        ATTACH_PARENT_PROCESS, AllocConsole, AttachConsole, GetStdHandle, STD_ERROR_HANDLE,
        STD_HANDLE, STD_OUTPUT_HANDLE,
    },
};

#[cfg(windows)]
fn main() -> ExitCode {
    let args = std::env::args().collect::<Vec<_>>();
    let cli_args = args
        .iter()
        .enumerate()
        .filter_map(|(index, arg)| {
            if index != 0 && arg == "--gui" {
                None
            } else {
                Some(arg.clone())
            }
        })
        .collect::<Vec<_>>();

    if cli_args.len() > 1 {
        prepare_cli_console();
        return driveck_cli::run_with_args(&cli_args);
    }

    app::run();
    ExitCode::SUCCESS
}

#[cfg(windows)]
fn prepare_cli_console() {
    if std_handle_available(STD_OUTPUT_HANDLE) || std_handle_available(STD_ERROR_HANDLE) {
        return;
    }

    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            let _ = AllocConsole();
        }
    }
}

#[cfg(windows)]
fn std_handle_available(handle_kind: STD_HANDLE) -> bool {
    match unsafe { GetStdHandle(handle_kind) } {
        Ok(handle) => handle != HANDLE::default() && handle != INVALID_HANDLE_VALUE,
        Err(_) => false,
    }
}
