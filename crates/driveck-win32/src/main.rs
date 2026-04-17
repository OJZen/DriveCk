#[cfg(not(windows))]
fn main() {
    eprintln!("The Win32 frontend is only available on Windows.");
}

#[cfg(windows)]
mod app {
    #![allow(unsafe_op_in_unsafe_fn)]

    use std::{
        mem::size_of,
        ptr::{null, null_mut},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread::{self, JoinHandle},
    };

    use driveck_core::{
        ProgressUpdate, TargetInfo, ValidationFailure, ValidationOptions, ValidationReport,
        collect_targets, discover_target, format_bytes, format_report_text, report_verdict,
        save_report, validate_target_with_callbacks,
    };
    use windows::{
        Win32::{
            Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
            System::{
                Com::{COINIT_APARTMENTTHREADED, CoInitializeEx},
                LibraryLoader::GetModuleHandleW,
            },
            UI::{
                Controls::{
                    Dialogs::{GetSaveFileNameW, OFN_EXPLORER, OFN_OVERWRITEPROMPT, OPENFILENAMEW},
                    ICC_PROGRESS_CLASS, INITCOMMONCONTROLSEX, InitCommonControlsEx, PBM_SETPOS,
                    PBM_SETRANGE32,
                },
                Input::KeyboardAndMouse::EnableWindow,
                WindowsAndMessaging::{
                    BS_PUSHBUTTON, CB_ADDSTRING, CB_GETCURSEL, CB_RESETCONTENT, CB_SETCURSEL,
                    CBN_SELCHANGE, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW,
                    DispatchMessageW, ES_AUTOVSCROLL, ES_LEFT, ES_MULTILINE, ES_READONLY,
                    ES_WANTRETURN, GWLP_USERDATA, GetClientRect, GetMessageW, GetWindowLongPtrW,
                    HMENU, IDC_ARROW, LoadCursorW, MB_ICONERROR, MB_ICONWARNING, MB_OK,
                    MB_OKCANCEL, MSG, MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW,
                    SendMessageW, SetWindowLongPtrW, SetWindowTextW, ShowWindow, TranslateMessage,
                    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CLOSE, WM_COMMAND, WM_CREATE,
                    WM_DESTROY, WM_NCCREATE, WNDCLASSW, WS_BORDER, WS_CHILD, WS_EX_CLIENTEDGE,
                    WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
                },
            },
        },
        core::{PCWSTR, PWSTR, w},
    };

    const IDC_DEVICE_COMBO: i32 = 100;
    const IDC_REFRESH: i32 = 101;
    const IDC_VALIDATE: i32 = 102;
    const IDC_STOP: i32 = 103;
    const IDC_SAVE: i32 = 104;
    const IDC_DETAILS: i32 = 105;
    const IDC_STATUS: i32 = 106;
    const IDC_PROGRESS: i32 = 107;
    const IDC_REPORT: i32 = 108;

    const WM_DRIVECK_PROGRESS: u32 = WM_APP + 1;
    const WM_DRIVECK_FINISHED: u32 = WM_APP + 2;

    struct AppState {
        hwnd: HWND,
        device_combo: HWND,
        refresh_button: HWND,
        validate_button: HWND,
        stop_button: HWND,
        save_button: HWND,
        details_label: HWND,
        status_label: HWND,
        progress_bar: HWND,
        report_edit: HWND,
        device_targets: Vec<TargetInfo>,
        report_text: Option<String>,
        last_target: Option<TargetInfo>,
        last_report: Option<ValidationReport>,
        worker: Option<JoinHandle<()>>,
        cancel_requested: Arc<AtomicBool>,
        closing_requested: bool,
    }

    struct ProgressPayload {
        phase: String,
        current: usize,
        total: usize,
    }

    struct FinishedPayload {
        target: TargetInfo,
        report: Option<ValidationReport>,
        report_text: Option<String>,
        error: Option<String>,
    }

    pub fn run() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let hinstance = HINSTANCE(GetModuleHandleW(None).unwrap().0);
            let class_name = w!("DriveCkWin32");
            let wc = WNDCLASSW {
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
                hInstance: hinstance,
                lpszClassName: class_name,
                lpfnWndProc: Some(window_proc),
                ..Default::default()
            };
            let _ = RegisterClassW(&wc);

            let icc = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_PROGRESS_CLASS,
            };
            let _ = InitCommonControlsEx(&icc);

            let hwnd = CreateWindowExW(
                Default::default(),
                class_name,
                w!("DriveCk"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                980,
                760,
                None,
                None,
                Some(hinstance),
                Some(null_mut()),
            )
            .expect("create main window");

            let _ = ShowWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::SW_SHOW);

            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).into() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_NCCREATE => {
                let _ = lparam;
                LRESULT(1)
            }
            WM_CREATE => {
                create_state(hwnd);
                if let Some(state) = state_mut(hwnd) {
                    refresh_devices(state);
                    update_actions(state);
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                handle_command(hwnd, wparam);
                LRESULT(0)
            }
            WM_DRIVECK_PROGRESS => {
                let payload = Box::from_raw(lparam.0 as *mut ProgressPayload);
                if let Some(state) = state_mut(hwnd) {
                    let fraction = if payload.total == 0 {
                        0
                    } else {
                        ((payload.current * 1000) / payload.total) as isize
                    };
                    send_message(state.progress_bar, PBM_SETPOS, fraction, 0);
                    let text = format!("{} {}/{}", payload.phase, payload.current, payload.total);
                    set_text(state.status_label, &text);
                }
                LRESULT(0)
            }
            WM_DRIVECK_FINISHED => {
                let payload = Box::from_raw(lparam.0 as *mut FinishedPayload);
                if let Some(state) = state_mut(hwnd) {
                    if let Some(worker) = state.worker.take() {
                        let _ = worker.join();
                    }
                    state.cancel_requested.store(false, Ordering::Relaxed);
                    state.last_target = Some(payload.target.clone());
                    state.last_report = payload.report.clone();
                    state.report_text = payload.report_text.clone();
                    if let Some(text) = payload.report_text.as_deref() {
                        set_text(state.report_edit, text);
                    }
                    if let Some(report) = payload.report.as_ref() {
                        let position = ((report.completed_samples * 1000)
                            / driveck_core::DRIVECK_SAMPLE_COUNT)
                            as isize;
                        send_message(state.progress_bar, PBM_SETPOS, position, 0);
                        let text = if let Some(error) = payload.error.as_deref() {
                            if report.cancelled {
                                "Validation cancelled.".to_string()
                            } else {
                                error.to_string()
                            }
                        } else {
                            format!("Finished: {}", report_verdict(report))
                        };
                        set_text(state.status_label, &text);
                    } else if let Some(error) = payload.error.as_deref() {
                        show_message(hwnd, "Validation failed.", error, MB_ICONERROR);
                        set_text(state.status_label, error);
                    }
                    update_actions(state);
                    if state.closing_requested {
                        PostQuitMessage(0);
                    }
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                if let Some(state) = state_mut(hwnd) {
                    if state.worker.is_some() {
                        state.closing_requested = true;
                        state.cancel_requested.store(true, Ordering::Relaxed);
                        set_text(state.status_label, "Stopping before exit...");
                        return LRESULT(0);
                    }
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_DESTROY => {
                if let Some(state_ptr) = take_state(hwnd) {
                    drop(Box::from_raw(state_ptr));
                }
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    unsafe fn create_state(hwnd: HWND) {
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);

        let device_combo = create_control(
            "COMBOBOX",
            "",
            hwnd,
            16,
            48,
            520,
            400,
            IDC_DEVICE_COMBO,
            WS_TABSTOP,
        );
        let refresh_button = create_control(
            "BUTTON",
            "Refresh devices",
            hwnd,
            556,
            48,
            130,
            28,
            IDC_REFRESH,
            ws(BS_PUSHBUTTON),
        );
        let validate_button = create_control(
            "BUTTON",
            "Validate",
            hwnd,
            696,
            48,
            120,
            28,
            IDC_VALIDATE,
            ws(BS_PUSHBUTTON),
        );
        let stop_button = create_control(
            "BUTTON",
            "Stop",
            hwnd,
            826,
            48,
            120,
            28,
            IDC_STOP,
            ws(BS_PUSHBUTTON),
        );
        let save_button = create_control(
            "BUTTON",
            "Save report...",
            hwnd,
            556,
            84,
            160,
            28,
            IDC_SAVE,
            ws(BS_PUSHBUTTON),
        );
        let details_label = create_control(
            "STATIC",
            "",
            hwnd,
            16,
            92,
            520,
            82,
            IDC_DETAILS,
            WINDOW_STYLE(0),
        );
        let status_label = create_control(
            "STATIC",
            "Ready.",
            hwnd,
            16,
            184,
            520,
            20,
            IDC_STATUS,
            WINDOW_STYLE(0),
        );
        let progress_bar = create_control(
            "msctls_progress32",
            "",
            hwnd,
            16,
            210,
            930,
            24,
            IDC_PROGRESS,
            WINDOW_STYLE(0),
        );
        let report_edit = create_control_ex(
            "EDIT",
            "No validation has run yet.\r\n\r\nChoose a removable or USB whole-disk device, then start validation.",
            hwnd,
            16,
            246,
            rect.right - 32,
            rect.bottom - 262,
            IDC_REPORT,
            ws(ES_LEFT | ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY | ES_WANTRETURN) | WS_BORDER,
            WS_EX_CLIENTEDGE,
        );
        send_message(progress_bar, PBM_SETRANGE32, 0, 1000);

        let state = Box::new(AppState {
            hwnd,
            device_combo,
            refresh_button,
            validate_button,
            stop_button,
            save_button,
            details_label,
            status_label,
            progress_bar,
            report_edit,
            device_targets: Vec::new(),
            report_text: None,
            last_target: None,
            last_report: None,
            worker: None,
            cancel_requested: Arc::new(AtomicBool::new(false)),
            closing_requested: false,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
    }

    unsafe fn handle_command(hwnd: HWND, wparam: WPARAM) {
        let code = ((wparam.0 >> 16) & 0xffff) as u16;
        let control_id = (wparam.0 & 0xffff) as i32;
        let Some(state) = state_mut(hwnd) else {
            return;
        };

        match control_id {
            IDC_REFRESH => refresh_devices(state),
            IDC_VALIDATE => start_validation(state),
            IDC_STOP => {
                if state.worker.is_some() {
                    state.cancel_requested.store(true, Ordering::Relaxed);
                    set_text(state.status_label, "Stopping...");
                }
            }
            IDC_SAVE => save_current_report(state),
            IDC_DEVICE_COMBO if code as u32 == CBN_SELCHANGE => update_device_details(state),
            _ => {}
        }
    }

    unsafe fn start_validation(state: &mut AppState) {
        let target = match prepare_selected_target(state) {
            Ok(target) => target,
            Err(error) => {
                show_message(
                    state.hwnd,
                    "Cannot start validation.",
                    &error,
                    MB_ICONWARNING,
                );
                return;
            }
        };

        let detail = format!(
            "DriveCk will temporarily overwrite sampled regions on {} ({}, {}{}). Continue?",
            target.path,
            format_bytes(target.size_bytes),
            target.vendor,
            if target.model.is_empty() {
                String::new()
            } else {
                format!(" {}", target.model)
            }
        );
        let response = MessageBoxW(
            Some(state.hwnd),
            PCWSTR(wide(&detail).as_ptr()),
            w!("Validate block device?"),
            MB_ICONWARNING | MB_OKCANCEL,
        );
        if response != windows::Win32::UI::WindowsAndMessaging::IDOK {
            return;
        }

        state.cancel_requested.store(false, Ordering::Relaxed);
        state.closing_requested = false;
        state.last_target = None;
        state.last_report = None;
        state.report_text = None;
        set_text(state.report_edit, "Validation in progress...");
        set_text(state.status_label, "Starting validation...");
        send_message(state.progress_bar, PBM_SETPOS, 0, 0);
        update_actions(state);

        let hwnd_raw = state.hwnd.0 as isize;
        let cancel_requested = state.cancel_requested.clone();
        state.worker = Some(thread::spawn(move || {
            let hwnd = HWND(hwnd_raw as *mut core::ffi::c_void);
            let mut progress = |update: ProgressUpdate| {
                let payload = Box::new(ProgressPayload {
                    phase: update.phase.to_string(),
                    current: update.current,
                    total: update.total,
                });
                unsafe {
                    let _ = PostMessageW(
                        Some(hwnd),
                        WM_DRIVECK_PROGRESS,
                        WPARAM(0),
                        LPARAM(Box::into_raw(payload) as isize),
                    );
                }
            };
            let cancel = || cancel_requested.load(Ordering::Relaxed);
            let result = validate_target_with_callbacks(
                &target,
                &ValidationOptions { seed: None },
                Some(&mut progress),
                Some(&cancel),
            );
            let payload = Box::new(build_finished_payload(target, result));
            unsafe {
                let _ = PostMessageW(
                    Some(hwnd),
                    WM_DRIVECK_FINISHED,
                    WPARAM(0),
                    LPARAM(Box::into_raw(payload) as isize),
                );
            }
        }));
    }

    fn build_finished_payload(
        target: TargetInfo,
        result: Result<ValidationReport, ValidationFailure>,
    ) -> FinishedPayload {
        match result {
            Ok(report) => FinishedPayload {
                report_text: Some(format_report_text(&target, &report)),
                target,
                report: Some(report),
                error: None,
            },
            Err(error) => FinishedPayload {
                report_text: error
                    .report
                    .as_ref()
                    .map(|report| format_report_text(&target, report)),
                target,
                report: error.report,
                error: Some(error.message),
            },
        }
    }

    unsafe fn prepare_selected_target(state: &AppState) -> Result<TargetInfo, String> {
        let selected = send_message(state.device_combo, CB_GETCURSEL, 0, 0).0 as usize;
        let path = state
            .device_targets
            .get(selected)
            .map(|target| target.path.clone())
            .ok_or_else(|| "Choose a removable or USB device first.".to_string())?;
        discover_target(&path).map_err(|error| error.message)
    }

    unsafe fn refresh_devices(state: &mut AppState) {
        let targets = match collect_targets() {
            Ok(targets) => targets,
            Err(error) => {
                show_message(
                    state.hwnd,
                    "Failed to refresh devices.",
                    &error.message,
                    MB_ICONERROR,
                );
                return;
            }
        };

        send_message(state.device_combo, CB_RESETCONTENT, 0, 0);
        for target in &targets {
            let row = wide(&format!(
                "{}  {}  {}{}{}{}",
                target.path,
                format_bytes(target.size_bytes),
                target.vendor,
                if !target.vendor.is_empty() && !target.model.is_empty() {
                    " "
                } else {
                    ""
                },
                target.model,
                if target.is_mounted { " [mounted]" } else { "" }
            ));
            send_message(state.device_combo, CB_ADDSTRING, 0, row.as_ptr() as isize);
        }
        state.device_targets = targets;
        send_message(state.device_combo, CB_SETCURSEL, 0, 0);
        update_device_details(state);
    }

    unsafe fn update_device_details(state: &AppState) {
        let selected = send_message(state.device_combo, CB_GETCURSEL, 0, 0).0 as usize;
        if let Some(target) = state.device_targets.get(selected) {
            let detail = format!(
                "Path: {}\r\nSize: {}\r\nTransport: {}{}\r\nModel: {}{}\r\nState: {}",
                target.path,
                format_bytes(target.size_bytes),
                if target.is_usb { "usb" } else { "block" },
                if target.is_removable {
                    ", removable"
                } else {
                    ""
                },
                target.vendor,
                if target.model.is_empty() {
                    String::new()
                } else {
                    format!(" {}", target.model)
                },
                if target.is_mounted {
                    "mounted, do not validate"
                } else {
                    "ready"
                }
            );
            set_text(state.details_label, &detail);
        } else {
            set_text(
                state.details_label,
                "No removable or USB whole-disk device is currently available.",
            );
        }
        update_actions(state);
    }

    unsafe fn save_current_report(state: &mut AppState) {
        let (Some(target), Some(report)) = (state.last_target.clone(), state.last_report.clone())
        else {
            return;
        };
        if let Some(path) = pick_save_path() {
            if let Err(error) = save_report(&path, &target, &report) {
                show_message(
                    state.hwnd,
                    "Failed to save report.",
                    &error.message,
                    MB_ICONERROR,
                );
            } else {
                set_text(state.status_label, "Report saved.");
            }
        }
    }

    unsafe fn update_actions(state: &AppState) {
        let busy = state.worker.is_some();
        let selected = send_message(state.device_combo, CB_GETCURSEL, 0, 0).0 as usize;
        let can_validate = state
            .device_targets
            .get(selected)
            .is_some_and(|target| !target.is_mounted)
            && !busy;

        enable(
            state.device_combo,
            !busy && !state.device_targets.is_empty(),
        );
        enable(state.refresh_button, !busy);
        enable(state.validate_button, can_validate);
        enable(state.stop_button, busy);
        enable(
            state.save_button,
            !busy && state.last_report.is_some() && state.report_text.is_some(),
        );
    }

    unsafe fn pick_save_path() -> Option<String> {
        let mut buffer = [0u16; 4096];
        let mut ofn = OPENFILENAMEW::default();
        ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
        ofn.lpstrFile = PWSTR(buffer.as_mut_ptr());
        ofn.nMaxFile = buffer.len() as u32;
        ofn.Flags = OFN_EXPLORER | OFN_OVERWRITEPROMPT;

        if !GetSaveFileNameW(&mut ofn).as_bool() {
            return None;
        }

        let len = buffer
            .iter()
            .position(|ch| *ch == 0)
            .unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..len]))
    }

    unsafe fn create_control(
        class_name: &str,
        text: &str,
        parent: HWND,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        id: i32,
        extra_style: WINDOW_STYLE,
    ) -> HWND {
        create_control_ex(
            class_name,
            text,
            parent,
            x,
            y,
            width,
            height,
            id,
            extra_style,
            Default::default(),
        )
    }

    unsafe fn create_control_ex(
        class_name: &str,
        text: &str,
        parent: HWND,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        id: i32,
        extra_style: WINDOW_STYLE,
        ex_style: WINDOW_EX_STYLE,
    ) -> HWND {
        let class_name = wide(class_name);
        let text = wide(text);
        CreateWindowExW(
            ex_style,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(text.as_ptr()),
            WS_CHILD | WS_VISIBLE | extra_style,
            x,
            y,
            width,
            height,
            Some(parent),
            Some(control_hmenu(id)),
            Some(HINSTANCE(GetModuleHandleW(None).unwrap().0)),
            Some(null()),
        )
        .expect("create child control")
    }

    unsafe fn set_text(hwnd: HWND, text: &str) {
        let wide = wide(text);
        let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
    }

    unsafe fn show_message(
        hwnd: HWND,
        title: &str,
        detail: &str,
        flags: windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE,
    ) {
        let title = wide(title);
        let detail = wide(detail);
        let _ = MessageBoxW(
            Some(hwnd),
            PCWSTR(detail.as_ptr()),
            PCWSTR(title.as_ptr()),
            flags | MB_OK,
        );
    }

    unsafe fn enable(hwnd: HWND, enabled: bool) {
        let _ = EnableWindow(hwnd, enabled);
    }

    unsafe fn send_message(hwnd: HWND, msg: u32, wparam: isize, lparam: isize) -> LRESULT {
        SendMessageW(
            hwnd,
            msg,
            Some(WPARAM(wparam as usize)),
            Some(LPARAM(lparam)),
        )
    }

    const fn ws(bits: i32) -> WINDOW_STYLE {
        WINDOW_STYLE(bits as u32)
    }

    fn control_hmenu(id: i32) -> HMENU {
        HMENU(id as usize as *mut core::ffi::c_void)
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe fn state_mut(hwnd: HWND) -> Option<&'static mut AppState> {
        let value = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if value == 0 {
            None
        } else {
            Some(&mut *(value as *mut AppState))
        }
    }

    unsafe fn take_state(hwnd: HWND) -> Option<*mut AppState> {
        let value = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if value == 0 {
            None
        } else {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            Some(value as *mut AppState)
        }
    }
}

#[cfg(windows)]
fn main() {
    app::run();
}
