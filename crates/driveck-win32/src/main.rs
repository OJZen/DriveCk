#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("The Win32 frontend is only available on Windows.");
}

#[cfg(windows)]
mod app {
    #![allow(unsafe_op_in_unsafe_fn)]

    use std::{
        ffi::{CStr, CString, c_char, c_void},
        fs,
        mem::size_of,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread::{self, JoinHandle},
    };

    use driveck_ffi::{
        driveck_ffi_format_report_text_json, driveck_ffi_free_string,
        driveck_ffi_inspect_target_json, driveck_ffi_list_targets_json,
        driveck_ffi_unmount_target_json, driveck_ffi_validate_target_json,
    };
    use serde::{Deserialize, Serialize, de::DeserializeOwned};
    use windows::{
        Win32::{
            Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
            Graphics::Gdi::{
                ANSI_FIXED_FONT, BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
                CreateSolidBrush, DEFAULT_GUI_FONT, DRAW_TEXT_FORMAT, DT_CENTER, DT_END_ELLIPSIS,
                DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, DeleteDC,
                DeleteObject, DrawTextW, EndPaint, FillRect, FrameRect, GetStockObject, HDC,
                HGDIOBJ, InvalidateRect, PAINTSTRUCT, SRCCOPY, SelectObject, SetBkMode,
                SetTextColor, TRANSPARENT,
            },
            System::{
                Com::{COINIT_APARTMENTTHREADED, CoInitializeEx},
                LibraryLoader::GetModuleHandleW,
            },
            UI::{
                Controls::{
                    Dialogs::{GetSaveFileNameW, OFN_EXPLORER, OFN_OVERWRITEPROMPT, OPENFILENAMEW},
                    ICC_PROGRESS_CLASS, INITCOMMONCONTROLSEX, InitCommonControlsEx, PBM_SETPOS,
                    PBM_SETRANGE32, SetWindowTheme,
                },
                Input::KeyboardAndMouse::EnableWindow,
                WindowsAndMessaging::{
                    BS_PUSHBUTTON, CB_ADDSTRING, CB_DELETESTRING, CB_GETCURSEL, CB_INSERTSTRING,
                    CB_RESETCONTENT, CB_SETCURSEL, CBN_SELCHANGE, CBS_DROPDOWNLIST, CW_USEDEFAULT,
                    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
                    ES_AUTOVSCROLL, ES_LEFT, ES_MULTILINE, ES_READONLY, ES_WANTRETURN,
                    GWLP_USERDATA, GetClientRect, GetMessageW, GetWindowLongPtrW, HMENU, IDC_ARROW,
                    IDOK, LoadCursorW, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_OKCANCEL,
                    MINMAXINFO, MSG, MessageBoxW, MoveWindow, PM_REMOVE, PeekMessageW,
                    PostMessageW, PostQuitMessage, RegisterClassW, SendMessageW, SetWindowLongPtrW,
                    SetWindowTextW, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE,
                    WM_APP, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_ERASEBKGND,
                    WM_GETMINMAXINFO, WM_NCCREATE, WM_PAINT, WM_SETFONT, WM_SIZE, WNDCLASSW,
                    WS_BORDER, WS_CHILD, WS_CLIPCHILDREN, WS_EX_CLIENTEDGE, WS_HSCROLL,
                    WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
                },
            },
        },
        core::{PCWSTR, PWSTR},
    };

    const IDC_DEVICE_COMBO: i32 = 100;
    const IDC_REFRESH: i32 = 101;
    const IDC_VALIDATE: i32 = 102;
    const IDC_STOP: i32 = 103;
    const IDC_SAVE: i32 = 104;
    const IDC_PROGRESS: i32 = 105;
    const IDC_REPORT: i32 = 106;

    const WM_DRIVECK_PROGRESS: u32 = WM_APP + 1;
    const WM_DRIVECK_FINISHED: u32 = WM_APP + 2;
    const CB_SETMINVISIBLE: u32 = 0x1701;

    const GRID_ROWS: usize = 18;
    const GRID_COLUMNS: usize = 32;
    const GRID_SAMPLES: usize = GRID_ROWS * GRID_COLUMNS;
    const MIN_WINDOW_WIDTH: i32 = 1100;
    const MIN_WINDOW_HEIGHT: i32 = 820;

    const APP_BG: COLORREF = rgb(244, 247, 251);
    const PANEL_BG: COLORREF = rgb(255, 255, 255);
    const PANEL_BORDER: COLORREF = rgb(217, 223, 230);
    const TEXT_PRIMARY: COLORREF = rgb(16, 24, 40);
    const TEXT_MUTED: COLORREF = rgb(102, 112, 133);
    const CHIP_NEUTRAL_BG: COLORREF = rgb(238, 242, 246);
    const CHIP_NEUTRAL_FG: COLORREF = rgb(71, 84, 103);
    const CHIP_SUCCESS_BG: COLORREF = rgb(231, 246, 236);
    const CHIP_SUCCESS_FG: COLORREF = rgb(22, 121, 68);
    const CHIP_DANGER_BG: COLORREF = rgb(253, 235, 236);
    const CHIP_DANGER_FG: COLORREF = rgb(196, 50, 63);
    const MAP_PENDING: COLORREF = rgb(214, 224, 238);
    const MAP_OK: COLORREF = rgb(31, 171, 99);
    const MAP_FAIL: COLORREF = rgb(219, 71, 74);
    const MAP_BG: COLORREF = rgb(246, 248, 251);
    const MAP_HIGHLIGHT: COLORREF = rgb(255, 255, 255);

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
    enum TargetKind {
        #[default]
        BlockDevice,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
    enum SampleStatus {
        #[default]
        Untested,
        Ok,
        ReadError,
        WriteError,
        VerifyMismatch,
        RestoreError,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    struct ValidationOptions {
        #[serde(default)]
        seed: Option<u64>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    struct TargetInfo {
        #[serde(default)]
        kind: TargetKind,
        #[serde(default)]
        path: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        vendor: String,
        #[serde(default)]
        model: String,
        #[serde(default)]
        transport: String,
        #[serde(default)]
        size_bytes: u64,
        #[serde(default)]
        logical_block_size: u32,
        #[serde(default)]
        is_block_device: bool,
        #[serde(default)]
        is_removable: bool,
        #[serde(default)]
        is_usb: bool,
        #[serde(default)]
        is_mounted: bool,
        #[serde(default)]
        direct_io: bool,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    struct TimingSeries {
        #[serde(default)]
        values: Vec<f64>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    struct ValidationReport {
        #[serde(default)]
        started_at: i64,
        #[serde(default)]
        finished_at: i64,
        #[serde(default)]
        seed: u64,
        #[serde(default)]
        reported_size_bytes: u64,
        #[serde(default)]
        region_size_bytes: u64,
        #[serde(default)]
        validated_drive_size_bytes: u64,
        #[serde(default)]
        highest_valid_region_bytes: u64,
        #[serde(default)]
        sample_offsets: Vec<u64>,
        #[serde(default)]
        sample_status: Vec<SampleStatus>,
        #[serde(default)]
        read_timings: TimingSeries,
        #[serde(default)]
        write_timings: TimingSeries,
        #[serde(default)]
        success_count: usize,
        #[serde(default)]
        read_error_count: usize,
        #[serde(default)]
        write_error_count: usize,
        #[serde(default)]
        mismatch_count: usize,
        #[serde(default)]
        restore_error_count: usize,
        #[serde(default)]
        completed_samples: usize,
        #[serde(default)]
        cancelled: bool,
        #[serde(default)]
        completed_all_samples: bool,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct ValidationResponse {
        target: TargetInfo,
        report: ValidationReport,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    struct ValidationRequest {
        target: TargetInfo,
        #[serde(default)]
        options: ValidationOptions,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    struct ValidationExecutionResult {
        #[serde(default)]
        response: Option<ValidationResponse>,
        #[serde(default)]
        error: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct Envelope<T> {
        ok: bool,
        data: Option<T>,
        error: Option<String>,
    }

    #[derive(Clone, Copy)]
    enum Tone {
        Neutral,
        Success,
        Danger,
    }

    struct ValidationGridState {
        sample_status: Vec<SampleStatus>,
        last_sample: Option<usize>,
    }

    impl Default for ValidationGridState {
        fn default() -> Self {
            Self {
                sample_status: vec![SampleStatus::Untested; GRID_SAMPLES],
                last_sample: None,
            }
        }
    }

    impl ValidationGridState {
        fn reset(&mut self) {
            self.sample_status.fill(SampleStatus::Untested);
            self.last_sample = None;
        }

        fn mark(&mut self, sample_index: usize, status: SampleStatus) {
            if let Some(slot) = self.sample_status.get_mut(sample_index) {
                *slot = status;
                self.last_sample = Some(sample_index);
            }
        }

        fn sync_from_report(&mut self, report: &ValidationReport) {
            self.sample_status.fill(SampleStatus::Untested);
            for (index, status) in report
                .sample_status
                .iter()
                .copied()
                .enumerate()
                .take(GRID_SAMPLES)
            {
                self.sample_status[index] = status;
            }
            self.last_sample = None;
        }

        fn counts(&self) -> (usize, usize, usize) {
            let processed = self
                .sample_status
                .iter()
                .filter(|status| **status != SampleStatus::Untested)
                .count();
            let ok = self
                .sample_status
                .iter()
                .filter(|status| **status == SampleStatus::Ok)
                .count();
            let failed = processed.saturating_sub(ok);
            (processed, ok, failed)
        }
    }

    #[derive(Clone, Copy)]
    struct Layout {
        title: RECT,
        subtitle: RECT,
        combo: RECT,
        refresh_button: RECT,
        validate_button: RECT,
        stop_button: RECT,
        save_button: RECT,
        device_panel: RECT,
        map_panel: RECT,
        summary_panel: RECT,
        report_panel: RECT,
        report_edit: RECT,
        status: RECT,
        progress: RECT,
    }

    struct AppState {
        hwnd: HWND,
        device_combo: HWND,
        refresh_button: HWND,
        validate_button: HWND,
        stop_button: HWND,
        save_button: HWND,
        progress_bar: HWND,
        report_edit: HWND,
        ui_font: HGDIOBJ,
        mono_font: HGDIOBJ,
        device_targets: Vec<TargetInfo>,
        validation_grid_state: ValidationGridState,
        report_text: Option<String>,
        last_response: Option<ValidationResponse>,
        worker: Option<JoinHandle<()>>,
        cancel_requested: Arc<AtomicBool>,
        stop_requested: bool,
        closing_requested: bool,
        status_text: String,
        status_tone: Tone,
    }

    impl AppState {
        unsafe fn selected_index(&self) -> Option<usize> {
            let selected = send_message(self.device_combo, CB_GETCURSEL, 0, 0).0;
            (selected >= 0).then_some(selected as usize)
        }

        unsafe fn selected_target(&self) -> Option<&TargetInfo> {
            self.selected_index()
                .and_then(|selected| self.device_targets.get(selected))
        }

        fn set_status(&mut self, text: impl Into<String>, tone: Tone) {
            self.status_text = text.into();
            self.status_tone = tone;
        }

        fn is_busy(&self) -> bool {
            self.worker.is_some()
        }
    }

    struct ProgressPayload {
        phase: String,
        current: usize,
        total: usize,
        final_update: bool,
        sample_index: Option<usize>,
        sample_status: Option<SampleStatus>,
    }

    struct FinishedPayload {
        response: Option<ValidationResponse>,
        report_text: Option<String>,
        error: Option<String>,
    }

    struct WorkerContext {
        hwnd: HWND,
        cancel_requested: Arc<AtomicBool>,
    }

    pub fn run() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let hinstance = HINSTANCE(GetModuleHandleW(None).unwrap().0);
            let class_name = wide("DriveCkWin32");
            let wc = WNDCLASSW {
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
                hInstance: hinstance,
                lpszClassName: PCWSTR(class_name.as_ptr()),
                lpfnWndProc: Some(window_proc),
                ..Default::default()
            };
            let _ = RegisterClassW(&wc);

            let icc = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_PROGRESS_CLASS,
            };
            let _ = InitCommonControlsEx(&icc);

            let title = wide("DriveCk");
            let hwnd = CreateWindowExW(
                Default::default(),
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                1180,
                860,
                None,
                None,
                Some(hinstance),
                None,
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
            WM_NCCREATE => LRESULT(1),
            WM_CREATE => {
                create_state(hwnd);
                if let Some(state) = state_mut(hwnd) {
                    refresh_devices(state);
                    update_actions(state);
                }
                LRESULT(0)
            }
            WM_SIZE => {
                if let Some(state) = state_mut(hwnd) {
                    layout_child_controls(state);
                    let _ = InvalidateRect(Some(hwnd), None, true);
                }
                LRESULT(0)
            }
            WM_GETMINMAXINFO => {
                let info = &mut *(lparam.0 as *mut MINMAXINFO);
                info.ptMinTrackSize.x = MIN_WINDOW_WIDTH;
                info.ptMinTrackSize.y = MIN_WINDOW_HEIGHT;
                LRESULT(0)
            }
            WM_COMMAND => {
                handle_command(hwnd, wparam);
                LRESULT(0)
            }
            WM_DRIVECK_PROGRESS => {
                let payload = Box::from_raw(lparam.0 as *mut ProgressPayload);
                if let Some(state) = state_mut(hwnd) {
                    if let (Some(sample_index), Some(sample_status)) =
                        (payload.sample_index, payload.sample_status)
                    {
                        state
                            .validation_grid_state
                            .mark(sample_index, sample_status);
                    }

                    let fraction = if payload.total == 0 {
                        0
                    } else {
                        ((payload.current * 1000) / payload.total) as isize
                    };
                    send_message(state.progress_bar, PBM_SETPOS, fraction, 0);

                    let progress_text = format!("{}/{}", payload.current, payload.total);
                    let status_text = if payload.final_update {
                        format!("{} {}", payload.phase, progress_text)
                    } else {
                        format!("{} sample {}", payload.phase, progress_text)
                    };
                    state.set_status(status_text, Tone::Neutral);
                    let _ = InvalidateRect(Some(hwnd), None, true);
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
                    state.stop_requested = false;
                    state.last_response = payload.response.clone();
                    state.report_text = payload.report_text.clone();

                    if let Some(text) = payload.report_text.as_deref() {
                        set_text(state.report_edit, text);
                    } else {
                        set_text(
                            state.report_edit,
                            "No report preview is available for this validation result.",
                        );
                    }

                    if let Some(response) = payload.response.as_ref() {
                        state
                            .validation_grid_state
                            .sync_from_report(&response.report);
                        let position =
                            ((response.report.completed_samples * 1000) / GRID_SAMPLES) as isize;
                        send_message(state.progress_bar, PBM_SETPOS, position, 0);
                        let (status_text, tone) =
                            final_status_text(payload.error.as_deref(), &response.report);
                        state.set_status(status_text, tone);
                    } else if let Some(error) = payload.error.as_deref() {
                        state.set_status(error, Tone::Danger);
                        show_message(hwnd, "Validation failed.", error, MB_ICONERROR);
                        send_message(state.progress_bar, PBM_SETPOS, 0, 0);
                    } else {
                        state.set_status("Validation finished.", Tone::Neutral);
                    }

                    update_actions(state);
                    let _ = InvalidateRect(Some(hwnd), None, true);
                    if state.closing_requested {
                        let _ = DestroyWindow(hwnd);
                    }
                }
                LRESULT(0)
            }
            WM_PAINT => {
                if let Some(state) = state_mut(hwnd) {
                    paint_window(hwnd, state);
                    LRESULT(0)
                } else {
                    DefWindowProcW(hwnd, message, wparam, lparam)
                }
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_CLOSE => {
                if let Some(state) = state_mut(hwnd) {
                    if state.worker.is_some() {
                        state.closing_requested = true;
                        request_stop(state, "Stopping before exit...");
                        return LRESULT(0);
                    }
                }
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                if let Some(state_ptr) = take_state(hwnd) {
                    let mut state = Box::from_raw(state_ptr);
                    state.cancel_requested.store(true, Ordering::Relaxed);
                    if let Some(worker) = state.worker.take() {
                        let _ = worker.join();
                    }
                    drain_worker_messages(hwnd);
                    drop(state);
                }
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    unsafe fn create_state(hwnd: HWND) {
        let ui_font = GetStockObject(DEFAULT_GUI_FONT);
        let mono_font = GetStockObject(ANSI_FIXED_FONT);

        let device_combo = create_control(
            "COMBOBOX",
            "",
            hwnd,
            0,
            0,
            0,
            0,
            IDC_DEVICE_COMBO,
            ws(CBS_DROPDOWNLIST) | WS_TABSTOP | WS_VSCROLL,
        );
        let refresh_button = create_control(
            "BUTTON",
            "Refresh",
            hwnd,
            0,
            0,
            0,
            0,
            IDC_REFRESH,
            ws(BS_PUSHBUTTON),
        );
        let validate_button = create_control(
            "BUTTON",
            "Validate",
            hwnd,
            0,
            0,
            0,
            0,
            IDC_VALIDATE,
            ws(BS_PUSHBUTTON),
        );
        let stop_button = create_control(
            "BUTTON",
            "Stop",
            hwnd,
            0,
            0,
            0,
            0,
            IDC_STOP,
            ws(BS_PUSHBUTTON),
        );
        let save_button = create_control(
            "BUTTON",
            "Save report...",
            hwnd,
            0,
            0,
            0,
            0,
            IDC_SAVE,
            ws(BS_PUSHBUTTON),
        );
        let progress_bar = create_control(
            "msctls_progress32",
            "",
            hwnd,
            0,
            0,
            0,
            0,
            IDC_PROGRESS,
            WINDOW_STYLE(0),
        );
        let report_edit = create_control_ex(
            "EDIT",
            "No validation has run yet.\r\n\r\nChoose a removable or USB whole-disk device, then start validation.",
            hwnd,
            0,
            0,
            0,
            0,
            IDC_REPORT,
            ws(ES_LEFT | ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY | ES_WANTRETURN)
                | WS_BORDER
                | WS_VSCROLL
                | WS_HSCROLL,
            WS_EX_CLIENTEDGE,
        );

        let state = Box::new(AppState {
            hwnd,
            device_combo,
            refresh_button,
            validate_button,
            stop_button,
            save_button,
            progress_bar,
            report_edit,
            ui_font,
            mono_font,
            device_targets: Vec::new(),
            validation_grid_state: ValidationGridState::default(),
            report_text: None,
            last_response: None,
            worker: None,
            cancel_requested: Arc::new(AtomicBool::new(false)),
            stop_requested: false,
            closing_requested: false,
            status_text: "Select a device to begin.".to_string(),
            status_tone: Tone::Neutral,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);

        if let Some(state) = state_mut(hwnd) {
            apply_default_fonts(state);
            apply_visual_theme(state);
            let _ = send_message(state.device_combo, CB_SETMINVISIBLE, 12, 0);
            send_message(state.progress_bar, PBM_SETRANGE32, 0, 1000);
            layout_child_controls(state);
        }
    }

    unsafe fn apply_default_fonts(state: &AppState) {
        let _ = SendMessageW(
            state.device_combo,
            WM_SETFONT,
            Some(WPARAM(state.ui_font.0 as usize)),
            Some(LPARAM(1)),
        );
        let _ = SendMessageW(
            state.refresh_button,
            WM_SETFONT,
            Some(WPARAM(state.ui_font.0 as usize)),
            Some(LPARAM(1)),
        );
        let _ = SendMessageW(
            state.validate_button,
            WM_SETFONT,
            Some(WPARAM(state.ui_font.0 as usize)),
            Some(LPARAM(1)),
        );
        let _ = SendMessageW(
            state.stop_button,
            WM_SETFONT,
            Some(WPARAM(state.ui_font.0 as usize)),
            Some(LPARAM(1)),
        );
        let _ = SendMessageW(
            state.save_button,
            WM_SETFONT,
            Some(WPARAM(state.ui_font.0 as usize)),
            Some(LPARAM(1)),
        );
        let _ = SendMessageW(
            state.report_edit,
            WM_SETFONT,
            Some(WPARAM(state.mono_font.0 as usize)),
            Some(LPARAM(1)),
        );
    }

    unsafe fn apply_visual_theme(state: &AppState) {
        let theme = wide("Explorer");
        for hwnd in [
            state.device_combo,
            state.refresh_button,
            state.validate_button,
            state.stop_button,
            state.save_button,
            state.progress_bar,
            state.report_edit,
        ] {
            let _ = SetWindowTheme(hwnd, PCWSTR(theme.as_ptr()), PCWSTR::null());
        }
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
            IDC_STOP => request_stop(state, "Stopping..."),
            IDC_SAVE => save_current_report(state),
            IDC_DEVICE_COMBO if code as u32 == CBN_SELCHANGE => {
                update_actions(state);
                let _ = InvalidateRect(Some(hwnd), None, true);
            }
            _ => {}
        }
    }

    unsafe fn start_validation(state: &mut AppState) {
        let path = match state.selected_target() {
            Some(target) => target.path.clone(),
            None => {
                let error = "Choose a removable or USB device first.".to_string();
                state.set_status(&error, Tone::Danger);
                show_message(
                    state.hwnd,
                    "Cannot start validation.",
                    &error,
                    MB_ICONWARNING,
                );
                let _ = InvalidateRect(Some(state.hwnd), None, true);
                return;
            }
        };

        let target = match ffi_inspect_target(&path) {
            Ok(target) => target,
            Err(error) => {
                state.set_status(&error, Tone::Danger);
                show_message(
                    state.hwnd,
                    "Cannot start validation.",
                    &error,
                    MB_ICONWARNING,
                );
                let _ = InvalidateRect(Some(state.hwnd), None, true);
                return;
            }
        };

        let detail = format!(
            "{}\r\n\r\nContinue?",
            if target.is_mounted {
                format!(
                    "DriveCk will dismount every mounted volume on {} and temporarily overwrite sampled regions on {} ({}, {}).\r\n\r\nClose Explorer windows and any open files on the disk first.",
                    target.path,
                    target.path,
                    format_bytes(target.size_bytes),
                    device_display_name(&target)
                )
            } else {
                format!(
                    "DriveCk will temporarily overwrite sampled regions on {} ({}, {}).",
                    target.path,
                    format_bytes(target.size_bytes),
                    device_display_name(&target)
                )
            }
        );
        let detail_text = wide(&detail);
        let title = wide("Validate block device?");
        let response = MessageBoxW(
            Some(state.hwnd),
            PCWSTR(detail_text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_ICONWARNING | MB_OKCANCEL,
        );
        if response != IDOK {
            return;
        }

        let target = match prepare_target_for_validation(state, target) {
            Ok(target) => target,
            Err(error) => {
                state.set_status(&error, Tone::Danger);
                show_message(
                    state.hwnd,
                    "Cannot start validation.",
                    &error,
                    MB_ICONWARNING,
                );
                let _ = InvalidateRect(Some(state.hwnd), None, true);
                return;
            }
        };

        state.cancel_requested.store(false, Ordering::Relaxed);
        state.stop_requested = false;
        state.closing_requested = false;
        state.last_response = None;
        state.report_text = None;
        state.validation_grid_state.reset();
        state.set_status("Starting validation...", Tone::Neutral);
        set_text(state.report_edit, "Validation in progress...");
        send_message(state.progress_bar, PBM_SETPOS, 0, 0);
        update_actions(state);
        let _ = InvalidateRect(Some(state.hwnd), None, true);

        let hwnd_raw = state.hwnd.0 as isize;
        let cancel_requested = state.cancel_requested.clone();
        state.worker = Some(thread::spawn(move || {
            let hwnd = HWND(hwnd_raw as *mut c_void);
            let context = WorkerContext {
                hwnd,
                cancel_requested,
            };
            let result = ffi_validate_target(&target, &ValidationOptions::default(), &context);
            let payload = Box::new(build_finished_payload(target, result));
            unsafe { post_boxed_message(hwnd, WM_DRIVECK_FINISHED, payload) };
        }));
    }

    unsafe fn request_stop(state: &mut AppState, status_text: &str) {
        if state.worker.is_none() || state.stop_requested {
            return;
        }
        state.cancel_requested.store(true, Ordering::Relaxed);
        state.stop_requested = true;
        state.set_status(status_text, Tone::Neutral);
        update_actions(state);
        let _ = InvalidateRect(Some(state.hwnd), None, true);
    }

    fn build_finished_payload(
        target: TargetInfo,
        result: Result<ValidationExecutionResult, String>,
    ) -> FinishedPayload {
        match result {
            Ok(execution) => {
                let mut error = execution.error;
                let report_text = if let Some(response) = execution.response.as_ref() {
                    match ffi_render_report(response) {
                        Ok(text) => Some(text),
                        Err(render_error) => {
                            if error.is_none() {
                                error = Some(render_error.clone());
                            }
                            Some(report_error_preview(
                                &response.target,
                                &format!(
                                    "Failed to render the shared report preview.\r\n\r\n{render_error}"
                                ),
                            ))
                        }
                    }
                } else {
                    error
                        .as_deref()
                        .map(|message| report_error_preview(&target, message))
                };

                FinishedPayload {
                    response: execution.response,
                    report_text,
                    error,
                }
            }
            Err(error) => FinishedPayload {
                response: None,
                report_text: Some(report_error_preview(&target, &error)),
                error: Some(error),
            },
        }
    }

    unsafe fn prepare_target_for_validation(
        state: &mut AppState,
        mut target: TargetInfo,
    ) -> Result<TargetInfo, String> {
        if target.is_mounted {
            state.set_status("Unmounting selected disk...", Tone::Neutral);
            let _ = InvalidateRect(Some(state.hwnd), None, true);

            target = ffi_unmount_target(&target.path)?;
            sync_discovered_target(state, &target);
        }

        if target.is_mounted {
            return Err(
                "DriveCk could not dismount every volume on the selected disk. Close any open files and try again."
                    .to_string(),
            );
        }
        Ok(target)
    }

    unsafe fn refresh_devices(state: &mut AppState) {
        let previous_path = state.selected_target().map(|target| target.path.clone());
        let targets = match ffi_list_targets() {
            Ok(targets) => targets,
            Err(error) => {
                state.set_status(&error, Tone::Danger);
                show_message(
                    state.hwnd,
                    "Failed to refresh devices.",
                    &error,
                    MB_ICONERROR,
                );
                let _ = InvalidateRect(Some(state.hwnd), None, true);
                return;
            }
        };

        send_message(state.device_combo, CB_RESETCONTENT, 0, 0);
        for target in &targets {
            let row = wide(&device_row_text(target));
            send_message(state.device_combo, CB_ADDSTRING, 0, row.as_ptr() as isize);
        }

        state.device_targets = targets;
        if let Some(previous_path) = previous_path {
            if let Some(index) = state
                .device_targets
                .iter()
                .position(|target| target.path == previous_path)
            {
                send_message(state.device_combo, CB_SETCURSEL, index as isize, 0);
            } else if !state.device_targets.is_empty() {
                send_message(state.device_combo, CB_SETCURSEL, 0, 0);
            }
        } else if !state.device_targets.is_empty() {
            send_message(state.device_combo, CB_SETCURSEL, 0, 0);
        }

        if state.device_targets.is_empty() {
            state.set_status(
                "No removable or USB whole-disk device is currently available.",
                Tone::Neutral,
            );
        } else {
            state.set_status("Select a device to begin.", Tone::Neutral);
        }
        update_actions(state);
        let _ = InvalidateRect(Some(state.hwnd), None, true);
    }

    unsafe fn sync_discovered_target(state: &mut AppState, target: &TargetInfo) {
        let Some(index) = state
            .device_targets
            .iter()
            .position(|entry| entry.path == target.path)
        else {
            return;
        };

        state.device_targets[index] = target.clone();
        let row = wide(&device_row_text(target));
        send_message(state.device_combo, CB_DELETESTRING, index as isize, 0);
        send_message(
            state.device_combo,
            CB_INSERTSTRING,
            index as isize,
            row.as_ptr() as isize,
        );
        send_message(state.device_combo, CB_SETCURSEL, index as isize, 0);
        update_actions(state);
        let _ = InvalidateRect(Some(state.hwnd), None, true);
    }

    unsafe fn save_current_report(state: &mut AppState) {
        let Some(report_text) = state.report_text.as_deref() else {
            return;
        };
        if state.last_response.is_none() {
            return;
        }

        if let Some(path) = pick_save_path() {
            if let Err(error) = fs::write(&path, report_text) {
                let message = format!("Failed to write report {path}: {error}");
                state.set_status(&message, Tone::Danger);
                show_message(state.hwnd, "Failed to save report.", &message, MB_ICONERROR);
            } else {
                state.set_status("Report saved.", Tone::Success);
            }
            let _ = InvalidateRect(Some(state.hwnd), None, true);
        }
    }

    unsafe fn update_actions(state: &AppState) {
        let busy = state.is_busy();
        let can_validate = state.selected_target().is_some() && !busy;

        enable(
            state.device_combo,
            !busy && !state.device_targets.is_empty(),
        );
        enable(state.refresh_button, !busy);
        enable(state.validate_button, can_validate);
        enable(state.stop_button, busy && !state.stop_requested);
        enable(
            state.save_button,
            !busy && state.last_response.is_some() && state.report_text.is_some(),
        );
    }

    unsafe fn layout_child_controls(state: &AppState) {
        let layout = current_layout(state.hwnd);
        let _ = MoveWindow(
            state.device_combo,
            layout.combo.left,
            layout.combo.top,
            rect_width(layout.combo),
            rect_height(layout.combo),
            true,
        );
        let _ = MoveWindow(
            state.refresh_button,
            layout.refresh_button.left,
            layout.refresh_button.top,
            rect_width(layout.refresh_button),
            rect_height(layout.refresh_button),
            true,
        );
        let _ = MoveWindow(
            state.validate_button,
            layout.validate_button.left,
            layout.validate_button.top,
            rect_width(layout.validate_button),
            rect_height(layout.validate_button),
            true,
        );
        let _ = MoveWindow(
            state.stop_button,
            layout.stop_button.left,
            layout.stop_button.top,
            rect_width(layout.stop_button),
            rect_height(layout.stop_button),
            true,
        );
        let _ = MoveWindow(
            state.save_button,
            layout.save_button.left,
            layout.save_button.top,
            rect_width(layout.save_button),
            rect_height(layout.save_button),
            true,
        );
        let _ = MoveWindow(
            state.progress_bar,
            layout.progress.left,
            layout.progress.top,
            rect_width(layout.progress),
            rect_height(layout.progress),
            true,
        );
        let _ = MoveWindow(
            state.report_edit,
            layout.report_edit.left,
            layout.report_edit.top,
            rect_width(layout.report_edit),
            rect_height(layout.report_edit),
            true,
        );
    }

    unsafe fn pick_save_path() -> Option<String> {
        let mut buffer = [0u16; 4096];
        let filter: Vec<u16> = "Text reports (*.txt)\0*.txt\0All files (*.*)\0*.*\0\0"
            .encode_utf16()
            .collect();
        let def_ext = wide("txt");

        let mut ofn = OPENFILENAMEW::default();
        ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
        ofn.lpstrFile = PWSTR(buffer.as_mut_ptr());
        ofn.nMaxFile = buffer.len() as u32;
        ofn.lpstrFilter = PCWSTR(filter.as_ptr());
        ofn.lpstrDefExt = PCWSTR(def_ext.as_ptr());
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

    unsafe fn paint_window(hwnd: HWND, state: &AppState) {
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut paint);
        let layout = current_layout(hwnd);
        let client = current_client_rect(hwnd);
        if rect_width(client) <= 0 || rect_height(client) <= 0 {
            let _ = EndPaint(hwnd, &paint);
            return;
        }

        let back_dc = CreateCompatibleDC(Some(hdc));
        let bitmap = CreateCompatibleBitmap(hdc, rect_width(client), rect_height(client));
        let old_bitmap = SelectObject(back_dc, HGDIOBJ(bitmap.0));

        paint_scene(back_dc, state, &layout);
        let _ = BitBlt(
            hdc,
            0,
            0,
            rect_width(client),
            rect_height(client),
            Some(back_dc),
            0,
            0,
            SRCCOPY,
        );

        SelectObject(back_dc, old_bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(back_dc);
        let _ = EndPaint(hwnd, &paint);
    }

    unsafe fn paint_scene(hdc: HDC, state: &AppState, layout: &Layout) {
        let client = current_client_rect(state.hwnd);
        fill_rect_color(hdc, &client, APP_BG);

        paint_header(hdc, state, layout);
        paint_device_panel(hdc, state, &layout.device_panel);
        paint_map_panel(hdc, state, &layout.map_panel);
        paint_summary_panel(hdc, state, &layout.summary_panel);
        paint_report_panel(hdc, state, &layout.report_panel);
        paint_footer(hdc, state, layout);
    }

    unsafe fn paint_header(hdc: HDC, state: &AppState, layout: &Layout) {
        draw_text_block(
            hdc,
            layout.title,
            "DriveCk",
            TEXT_PRIMARY,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            state.ui_font,
        );
        draw_text_block(
            hdc,
            layout.subtitle,
            "Windows dashboard using the shared Rust FFI validation engine.",
            TEXT_MUTED,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
            state.ui_font,
        );
    }

    unsafe fn paint_device_panel(hdc: HDC, state: &AppState, panel: &RECT) {
        draw_panel(hdc, panel);
        let inner = inset_rect(*panel, 16, 16);
        let title_rect = make_rect(inner.left, inner.top, rect_width(inner), 20);
        draw_panel_title(hdc, &title_rect, "Device", state.ui_font);

        if let Some(target) = state.selected_target() {
            let name_rect = make_rect(inner.left, inner.top + 28, rect_width(inner), 24);
            let path_rect = make_rect(inner.left, inner.top + 52, rect_width(inner), 22);
            let chips_top = inner.top + 82;
            let chips = split_three(make_rect(inner.left, chips_top, rect_width(inner), 30), 8);
            let note_rect = make_rect(inner.left, chips_top + 40, rect_width(inner), 24);

            draw_text_block(
                hdc,
                name_rect,
                &device_display_name(target),
                TEXT_PRIMARY,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                state.ui_font,
            );
            draw_text_block(
                hdc,
                path_rect,
                &target.path,
                TEXT_MUTED,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                state.ui_font,
            );
            draw_chip(
                hdc,
                &chips[0],
                &format_bytes(target.size_bytes),
                Tone::Neutral,
                state.ui_font,
            );
            draw_chip(
                hdc,
                &chips[1],
                &device_transport_text(target),
                if target.is_usb || target.is_removable {
                    Tone::Success
                } else {
                    Tone::Neutral
                },
                state.ui_font,
            );
            draw_chip(
                hdc,
                &chips[2],
                if target.is_mounted {
                    "Mounted"
                } else {
                    "Ready"
                },
                if target.is_mounted {
                    Tone::Danger
                } else {
                    Tone::Success
                },
                state.ui_font,
            );
            draw_text_block(
                hdc,
                note_rect,
                if target.is_mounted {
                    "Validate will dismount every volume on this disk before starting. Close open files first."
                } else {
                    "DriveCk samples the whole disk and restores each tested region."
                },
                TEXT_MUTED,
                DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
                state.ui_font,
            );
        } else {
            let message_rect = make_rect(inner.left, inner.top + 34, rect_width(inner), 70);
            let note_rect = make_rect(inner.left, inner.top + 82, rect_width(inner), 30);
            draw_text_block(
                hdc,
                message_rect,
                "No removable or USB whole-disk device is currently available.",
                TEXT_PRIMARY,
                DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
                state.ui_font,
            );
            draw_text_block(
                hdc,
                note_rect,
                "Refresh the list after inserting or unmounting a target.",
                TEXT_MUTED,
                DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
                state.ui_font,
            );
        }
    }

    unsafe fn paint_map_panel(hdc: HDC, state: &AppState, panel: &RECT) {
        draw_panel(hdc, panel);
        let inner = inset_rect(*panel, 16, 16);
        let title_rect = make_rect(inner.left, inner.top, rect_width(inner), 20);
        draw_panel_title(hdc, &title_rect, "Validation map", state.ui_font);

        let metrics_rect = make_rect(inner.left, inner.bottom - 30, rect_width(inner), 30);
        let helper_rect = make_rect(inner.left, metrics_rect.top - 24, rect_width(inner), 18);
        let map_rect = make_rect(
            inner.left,
            title_rect.bottom + 12,
            rect_width(inner),
            helper_rect.top - title_rect.bottom - 20,
        );

        draw_validation_map(hdc, &map_rect, &state.validation_grid_state);

        draw_text_block(
            hdc,
            helper_rect,
            "Gray pending   Green valid   Red issue",
            TEXT_MUTED,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            state.ui_font,
        );

        let (processed, ok, failed) = state.validation_grid_state.counts();
        let chips = split_three(metrics_rect, 8);
        draw_chip(
            hdc,
            &chips[0],
            &format!("Done {processed}/{GRID_SAMPLES}"),
            Tone::Neutral,
            state.ui_font,
        );
        draw_chip(
            hdc,
            &chips[1],
            &format!("OK {ok}"),
            Tone::Success,
            state.ui_font,
        );
        draw_chip(
            hdc,
            &chips[2],
            &format!("Fail {failed}"),
            if failed == 0 {
                Tone::Neutral
            } else {
                Tone::Danger
            },
            state.ui_font,
        );
    }

    unsafe fn paint_summary_panel(hdc: HDC, state: &AppState, panel: &RECT) {
        draw_panel(hdc, panel);
        let inner = inset_rect(*panel, 16, 16);
        let title_rect = make_rect(inner.left, inner.top, rect_width(inner), 20);
        draw_panel_title(hdc, &title_rect, "Summary", state.ui_font);

        let chips_rect = make_rect(inner.left, inner.top + 28, rect_width(inner), 30);
        if let Some(response) = state.last_response.as_ref() {
            let failure_detail = format_failure_summary(&response.report);
            let chips = split_three(chips_rect, 8);
            let (failure_chip, failure_tone) = report_failure_chip(&response.report);
            draw_chip(
                hdc,
                &chips[0],
                report_verdict(&response.report),
                report_verdict_tone(&response.report),
                state.ui_font,
            );
            draw_chip(
                hdc,
                &chips[1],
                &format!(
                    "Samples {}/{}",
                    response.report.completed_samples, GRID_SAMPLES
                ),
                if response.report.completed_all_samples && !response.report.cancelled {
                    Tone::Success
                } else {
                    Tone::Neutral
                },
                state.ui_font,
            );
            draw_chip(hdc, &chips[2], &failure_chip, failure_tone, state.ui_font);

            let row1 = make_rect(inner.left, chips_rect.bottom + 14, rect_width(inner), 34);
            let row2 = make_rect(inner.left, row1.bottom + 8, rect_width(inner), 34);
            let row3 = make_rect(inner.left, row2.bottom + 8, rect_width(inner), 34);
            let row4 = make_rect(inner.left, row3.bottom + 8, rect_width(inner), 44);

            draw_summary_full_row(hdc, &row1, "Target", &response.target.path, state.ui_font);
            draw_summary_pair_row(
                hdc,
                &row2,
                "Reported",
                &format_bytes(response.report.reported_size_bytes),
                "Validated",
                &format_bytes(response.report.validated_drive_size_bytes),
                state.ui_font,
            );
            draw_summary_pair_row(
                hdc,
                &row3,
                "Highest valid",
                &format_bytes(response.report.highest_valid_region_bytes),
                "Region",
                &format_bytes(response.report.region_size_bytes),
                state.ui_font,
            );
            draw_summary_full_row(hdc, &row4, "Failure detail", &failure_detail, state.ui_font);
        } else {
            draw_chip(
                hdc,
                &chips_rect,
                "Summary unavailable",
                Tone::Neutral,
                state.ui_font,
            );
            let message_rect = make_rect(inner.left, chips_rect.bottom + 16, rect_width(inner), 80);
            let target_text = state
                .selected_target()
                .map(|target| target.path.as_str())
                .unwrap_or("-");
            let message = format!(
                "Run a validation to populate the report summary.\r\nTarget: {target_text}"
            );
            draw_text_block(
                hdc,
                message_rect,
                &message,
                TEXT_MUTED,
                DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
                state.ui_font,
            );
        }
    }

    unsafe fn paint_report_panel(hdc: HDC, state: &AppState, panel: &RECT) {
        draw_panel(hdc, panel);
        let inner = inset_rect(*panel, 16, 16);
        let title_rect = make_rect(inner.left, inner.top, rect_width(inner), 20);
        let hint_rect = make_rect(inner.left, inner.top + 24, rect_width(inner), 18);
        draw_panel_title(hdc, &title_rect, "Report preview", state.ui_font);
        draw_text_block(
            hdc,
            hint_rect,
            if state.report_text.is_some() {
                "Rendered by the shared Rust report formatter."
            } else {
                "The report preview fills in when a run starts."
            },
            TEXT_MUTED,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
            state.ui_font,
        );
    }

    unsafe fn paint_footer(hdc: HDC, state: &AppState, layout: &Layout) {
        draw_text_block(
            hdc,
            layout.status,
            &state.status_text,
            tone_text_color(state.status_tone),
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
            state.ui_font,
        );
    }

    unsafe fn draw_validation_map(hdc: HDC, map_bounds: &RECT, grid_state: &ValidationGridState) {
        draw_panel_surface(hdc, map_bounds, MAP_BG);

        let gap = 2;
        let available_width = rect_width(*map_bounds) - gap * (GRID_COLUMNS as i32 - 1) - 12;
        let available_height = rect_height(*map_bounds) - gap * (GRID_ROWS as i32 - 1) - 12;
        let cell_width = (available_width / GRID_COLUMNS as i32).max(1);
        let cell_height = (available_height / GRID_ROWS as i32).max(1);
        let cell = cell_width.min(cell_height);
        let grid_width = cell * GRID_COLUMNS as i32 + gap * (GRID_COLUMNS as i32 - 1);
        let grid_height = cell * GRID_ROWS as i32 + gap * (GRID_ROWS as i32 - 1);
        let origin_x = map_bounds.left + ((rect_width(*map_bounds) - grid_width) / 2);
        let origin_y = map_bounds.top + ((rect_height(*map_bounds) - grid_height) / 2);

        for row in 0..GRID_ROWS {
            for column in 0..GRID_COLUMNS {
                let index = row * GRID_COLUMNS + column;
                let cell_rect = make_rect(
                    origin_x + column as i32 * (cell + gap),
                    origin_y + row as i32 * (cell + gap),
                    cell,
                    cell,
                );
                let color = match grid_state.sample_status.get(index).copied() {
                    Some(SampleStatus::Untested) => MAP_PENDING,
                    Some(SampleStatus::Ok) => MAP_OK,
                    Some(
                        SampleStatus::ReadError
                        | SampleStatus::WriteError
                        | SampleStatus::VerifyMismatch
                        | SampleStatus::RestoreError,
                    ) => MAP_FAIL,
                    None => MAP_PENDING,
                };
                fill_rect_color(hdc, &cell_rect, color);
                if grid_state.last_sample == Some(index) {
                    frame_rect_color(hdc, &cell_rect, MAP_HIGHLIGHT);
                }
            }
        }
    }

    fn report_error_preview(target: &TargetInfo, error: &str) -> String {
        format!("DriveCk\r\nTarget: {}\r\n\r\n{}", target.path, error)
    }

    fn final_status_text(error: Option<&str>, report: &ValidationReport) -> (String, Tone) {
        if let Some(error) = error {
            if report.cancelled {
                ("Validation cancelled.".to_string(), Tone::Neutral)
            } else {
                (error.to_string(), Tone::Danger)
            }
        } else {
            (
                format!("Finished: {}", report_verdict(report)),
                report_verdict_tone(report),
            )
        }
    }

    fn report_issue_count(report: &ValidationReport) -> usize {
        report.read_error_count
            + report.write_error_count
            + report.mismatch_count
            + report.restore_error_count
    }

    fn report_verdict(report: &ValidationReport) -> &'static str {
        if report.restore_error_count != 0 {
            return "critical restore failure";
        }
        if report.cancelled {
            return "validation cancelled";
        }
        if report.mismatch_count != 0 {
            return "missing or spoofed storage detected";
        }
        if report.read_error_count != 0 || report.write_error_count != 0 {
            return "I/O errors detected";
        }
        if !report.completed_all_samples {
            return "validation incomplete";
        }
        "all sampled regions validated"
    }

    fn report_verdict_tone(report: &ValidationReport) -> Tone {
        if report_issue_count(report) != 0 {
            Tone::Danger
        } else if report.cancelled || !report.completed_all_samples {
            Tone::Neutral
        } else {
            Tone::Success
        }
    }

    fn report_failure_chip(report: &ValidationReport) -> (String, Tone) {
        let failure_count = report_issue_count(report);
        if failure_count != 0 {
            (format!("Failures {failure_count}"), Tone::Danger)
        } else if report.cancelled {
            ("Cancelled".to_string(), Tone::Neutral)
        } else if !report.completed_all_samples {
            ("Incomplete".to_string(), Tone::Neutral)
        } else {
            ("No failures".to_string(), Tone::Success)
        }
    }

    fn format_failure_summary(report: &ValidationReport) -> String {
        let mut parts = Vec::new();
        if report.read_error_count != 0 {
            parts.push(format!("read {}", report.read_error_count));
        }
        if report.write_error_count != 0 {
            parts.push(format!("write {}", report.write_error_count));
        }
        if report.mismatch_count != 0 {
            parts.push(format!("mismatch {}", report.mismatch_count));
        }
        if report.restore_error_count != 0 {
            parts.push(format!("restore {}", report.restore_error_count));
        }
        if report.cancelled {
            parts.push("cancelled".to_string());
        } else if !report.completed_all_samples {
            parts.push("incomplete".to_string());
        }
        if parts.is_empty() {
            "none".to_string()
        } else {
            parts.join(", ")
        }
    }

    fn format_bytes(bytes: u64) -> String {
        const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];

        let mut value = bytes as f64;
        let mut unit_index = 0usize;
        while value >= 1024.0 && unit_index + 1 < UNITS.len() {
            value /= 1024.0;
            unit_index += 1;
        }
        format!("{value:.2} {}", UNITS[unit_index])
    }

    fn device_display_name(target: &TargetInfo) -> String {
        let composite = format!("{} {}", target.vendor.trim(), target.model.trim())
            .trim()
            .to_string();
        if !composite.is_empty() {
            composite
        } else if !target.name.is_empty() {
            target.name.clone()
        } else {
            target.path.clone()
        }
    }

    fn device_transport_text(target: &TargetInfo) -> String {
        if target.is_usb && target.is_removable {
            "USB / Removable".to_string()
        } else if target.is_usb {
            "USB".to_string()
        } else if target.is_removable {
            "Removable".to_string()
        } else if !target.transport.is_empty() {
            target.transport.to_ascii_uppercase()
        } else {
            "Block device".to_string()
        }
    }

    fn device_row_text(target: &TargetInfo) -> String {
        format!(
            "{} | {} | {}{}",
            device_display_name(target),
            format_bytes(target.size_bytes),
            device_transport_text(target),
            if target.is_mounted { " | mounted" } else { "" }
        )
    }

    extern "C" fn ffi_progress_callback(
        phase: *const c_char,
        current: usize,
        total: usize,
        final_update: bool,
        sample_index: isize,
        sample_status: i32,
        user_data: *mut c_void,
    ) {
        if user_data.is_null() {
            return;
        }
        let context = unsafe { &*(user_data as *const WorkerContext) };
        let phase_text = if phase.is_null() {
            "Working".to_string()
        } else {
            unsafe { CStr::from_ptr(phase) }
                .to_string_lossy()
                .into_owned()
        };
        let payload = Box::new(ProgressPayload {
            phase: phase_text,
            current,
            total,
            final_update,
            sample_index: (sample_index >= 0).then_some(sample_index as usize),
            sample_status: sample_status_from_code(sample_status),
        });
        unsafe { post_boxed_message(context.hwnd, WM_DRIVECK_PROGRESS, payload) };
    }

    extern "C" fn ffi_cancel_callback(user_data: *mut c_void) -> bool {
        if user_data.is_null() {
            return false;
        }
        let context = unsafe { &*(user_data as *const WorkerContext) };
        context.cancel_requested.load(Ordering::Relaxed)
    }

    fn sample_status_from_code(code: i32) -> Option<SampleStatus> {
        match code {
            0 => Some(SampleStatus::Untested),
            1 => Some(SampleStatus::Ok),
            2 => Some(SampleStatus::ReadError),
            3 => Some(SampleStatus::WriteError),
            4 => Some(SampleStatus::VerifyMismatch),
            5 => Some(SampleStatus::RestoreError),
            _ => None,
        }
    }

    fn ffi_list_targets() -> Result<Vec<TargetInfo>, String> {
        decode_envelope(
            driveck_ffi_list_targets_json(),
            "driveck_ffi_list_targets_json",
        )
    }

    fn ffi_inspect_target(path: &str) -> Result<TargetInfo, String> {
        let path =
            CString::new(path).map_err(|_| "Device path contains a null byte.".to_string())?;
        decode_envelope(
            driveck_ffi_inspect_target_json(path.as_ptr()),
            "driveck_ffi_inspect_target_json",
        )
    }

    fn ffi_unmount_target(path: &str) -> Result<TargetInfo, String> {
        let path =
            CString::new(path).map_err(|_| "Device path contains a null byte.".to_string())?;
        decode_envelope(
            driveck_ffi_unmount_target_json(path.as_ptr()),
            "driveck_ffi_unmount_target_json",
        )
    }

    fn ffi_validate_target(
        target: &TargetInfo,
        options: &ValidationOptions,
        context: &WorkerContext,
    ) -> Result<ValidationExecutionResult, String> {
        let request = ValidationRequest {
            target: target.clone(),
            options: options.clone(),
        };
        let request = encode_json(&request, "validation request")?;
        decode_envelope(
            driveck_ffi_validate_target_json(
                request.as_ptr(),
                false,
                0,
                Some(ffi_progress_callback),
                Some(ffi_cancel_callback),
                context as *const WorkerContext as *mut c_void,
            ),
            "driveck_ffi_validate_target_json",
        )
    }

    fn ffi_render_report(response: &ValidationResponse) -> Result<String, String> {
        let response = encode_json(response, "validation response")?;
        decode_envelope(
            driveck_ffi_format_report_text_json(response.as_ptr()),
            "driveck_ffi_format_report_text_json",
        )
    }

    fn encode_json<T: Serialize>(value: &T, label: &str) -> Result<CString, String> {
        let text = serde_json::to_string(value)
            .map_err(|error| format!("Failed to encode {label} as JSON: {error}"))?;
        CString::new(text).map_err(|_| format!("{label} contains a null byte."))
    }

    fn decode_envelope<T: DeserializeOwned>(
        pointer: *mut c_char,
        label: &str,
    ) -> Result<T, String> {
        let text = take_ffi_string(pointer)?;
        let envelope: Envelope<T> = serde_json::from_str(&text)
            .map_err(|error| format!("{label} returned invalid JSON: {error}"))?;
        if !envelope.ok {
            return Err(envelope
                .error
                .unwrap_or_else(|| format!("{label} reported an unknown error.")));
        }
        envelope.data.ok_or_else(|| {
            envelope
                .error
                .unwrap_or_else(|| format!("{label} returned no data."))
        })
    }

    fn take_ffi_string(pointer: *mut c_char) -> Result<String, String> {
        if pointer.is_null() {
            return Err("DriveCk FFI returned a null string pointer.".to_string());
        }
        let bytes = unsafe { CStr::from_ptr(pointer) }.to_bytes().to_vec();
        driveck_ffi_free_string(pointer);
        String::from_utf8(bytes)
            .map_err(|_| "DriveCk FFI returned a non-UTF-8 JSON string.".to_string())
    }

    unsafe fn draw_panel(hdc: HDC, rect: &RECT) {
        fill_rect_color(hdc, rect, PANEL_BG);
        frame_rect_color(hdc, rect, PANEL_BORDER);
    }

    unsafe fn draw_panel_surface(hdc: HDC, rect: &RECT, background: COLORREF) {
        fill_rect_color(hdc, rect, background);
        frame_rect_color(hdc, rect, PANEL_BORDER);
    }

    unsafe fn draw_panel_title(hdc: HDC, rect: &RECT, text: &str, font: HGDIOBJ) {
        draw_text_block(
            hdc,
            *rect,
            text,
            TEXT_PRIMARY,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            font,
        );
    }

    unsafe fn draw_chip(hdc: HDC, rect: &RECT, text: &str, tone: Tone, font: HGDIOBJ) {
        let (background, foreground) = tone_palette(tone);
        fill_rect_color(hdc, rect, background);
        frame_rect_color(hdc, rect, background);
        draw_text_block(
            hdc,
            *rect,
            text,
            foreground,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
            font,
        );
    }

    unsafe fn draw_summary_full_row(hdc: HDC, rect: &RECT, key: &str, value: &str, font: HGDIOBJ) {
        let key_rect = make_rect(rect.left, rect.top, 110, rect_height(*rect));
        let value_rect = make_rect(
            rect.left + 118,
            rect.top,
            rect_width(*rect) - 118,
            rect_height(*rect),
        );
        draw_text_block(
            hdc,
            key_rect,
            key,
            TEXT_MUTED,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            font,
        );
        draw_text_block(
            hdc,
            value_rect,
            value,
            TEXT_PRIMARY,
            DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
            font,
        );
    }

    unsafe fn draw_summary_pair_row(
        hdc: HDC,
        rect: &RECT,
        left_key: &str,
        left_value: &str,
        right_key: &str,
        right_value: &str,
        font: HGDIOBJ,
    ) {
        let halves = split_two(*rect, 14);
        draw_summary_full_row(hdc, &halves[0], left_key, left_value, font);
        draw_summary_full_row(hdc, &halves[1], right_key, right_value, font);
    }

    unsafe fn draw_text_block(
        hdc: HDC,
        mut rect: RECT,
        text: &str,
        color: COLORREF,
        flags: DRAW_TEXT_FORMAT,
        font: HGDIOBJ,
    ) {
        if text.is_empty() {
            return;
        }
        let mut utf16 = utf16(text);
        SelectObject(hdc, font);
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, color);
        let _ = DrawTextW(hdc, utf16.as_mut_slice(), &mut rect, flags | DT_NOPREFIX);
    }

    unsafe fn fill_rect_color(hdc: HDC, rect: &RECT, color: COLORREF) {
        let brush = CreateSolidBrush(color);
        let _ = FillRect(hdc, rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }

    unsafe fn frame_rect_color(hdc: HDC, rect: &RECT, color: COLORREF) {
        let brush = CreateSolidBrush(color);
        let _ = FrameRect(hdc, rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }

    fn tone_palette(tone: Tone) -> (COLORREF, COLORREF) {
        match tone {
            Tone::Neutral => (CHIP_NEUTRAL_BG, CHIP_NEUTRAL_FG),
            Tone::Success => (CHIP_SUCCESS_BG, CHIP_SUCCESS_FG),
            Tone::Danger => (CHIP_DANGER_BG, CHIP_DANGER_FG),
        }
    }

    fn tone_text_color(tone: Tone) -> COLORREF {
        match tone {
            Tone::Neutral => TEXT_MUTED,
            Tone::Success => CHIP_SUCCESS_FG,
            Tone::Danger => CHIP_DANGER_FG,
        }
    }

    fn current_layout(hwnd: HWND) -> Layout {
        let client = current_client_rect(hwnd);
        let width = rect_width(client);
        let height = rect_height(client);
        let margin = 20;
        let section_gap = 16;
        let button_gap = 8;
        let title_height = 24;
        let subtitle_height = 18;
        let control_height = 32;
        let footer_status_height = 20;
        let footer_progress_height = 20;

        let title = make_rect(margin, 12, width - margin * 2, title_height);
        let subtitle = make_rect(
            margin,
            title.bottom + 2,
            width - margin * 2,
            subtitle_height,
        );

        let controls_top = subtitle.bottom + 16;
        let refresh_width = 108;
        let validate_width = 110;
        let stop_width = 96;
        let save_width = 132;
        let combo_width = (width
            - margin * 2
            - button_gap * 4
            - refresh_width
            - validate_width
            - stop_width
            - save_width)
            .max(280);

        let combo = make_rect(margin, controls_top, combo_width, control_height);
        let refresh_button = make_rect(
            combo.right + button_gap,
            controls_top,
            refresh_width,
            control_height,
        );
        let validate_button = make_rect(
            refresh_button.right + button_gap,
            controls_top,
            validate_width,
            control_height,
        );
        let stop_button = make_rect(
            validate_button.right + button_gap,
            controls_top,
            stop_width,
            control_height,
        );
        let save_button = make_rect(
            stop_button.right + button_gap,
            controls_top,
            save_width,
            control_height,
        );

        let footer_progress_top = height - margin - footer_progress_height;
        let footer_status_top = footer_progress_top - 4 - footer_status_height;
        let status = make_rect(
            margin,
            footer_status_top,
            width - margin * 2,
            footer_status_height,
        );
        let progress = make_rect(
            margin,
            footer_progress_top,
            width - margin * 2,
            footer_progress_height,
        );

        let content_top = controls_top + control_height + 18;
        let content_bottom = footer_status_top - section_gap;
        let content_height = content_bottom - content_top;
        let left_width = ((width - margin * 2 - section_gap) * 44 / 100).clamp(420, 500);
        let right_x = margin + left_width + section_gap;
        let right_width = width - right_x - margin;

        let mut device_height = 134;
        let mut map_height = 332;
        let min_summary_height = 190;
        let summary_height = content_height - device_height - map_height - section_gap * 2;
        if summary_height < min_summary_height {
            map_height -= min_summary_height - summary_height;
        }
        if map_height < 250 {
            let deficit = 250 - map_height;
            map_height = 250;
            device_height = (device_height - deficit).max(118);
        }
        let summary_height = content_height - device_height - map_height - section_gap * 2;

        let device_panel = make_rect(margin, content_top, left_width, device_height);
        let map_panel = make_rect(
            margin,
            device_panel.bottom + section_gap,
            left_width,
            map_height,
        );
        let summary_panel = make_rect(
            margin,
            map_panel.bottom + section_gap,
            left_width,
            summary_height.max(min_summary_height),
        );
        let report_panel = make_rect(right_x, content_top, right_width, content_height);
        let report_edit = make_rect(
            report_panel.left + 16,
            report_panel.top + 52,
            rect_width(report_panel) - 32,
            rect_height(report_panel) - 68,
        );

        Layout {
            title,
            subtitle,
            combo,
            refresh_button,
            validate_button,
            stop_button,
            save_button,
            device_panel,
            map_panel,
            summary_panel,
            report_panel,
            report_edit,
            status,
            progress,
        }
    }

    fn current_client_rect(hwnd: HWND) -> RECT {
        let mut rect = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rect);
        }
        rect
    }

    fn inset_rect(rect: RECT, dx: i32, dy: i32) -> RECT {
        RECT {
            left: rect.left + dx,
            top: rect.top + dy,
            right: rect.right - dx,
            bottom: rect.bottom - dy,
        }
    }

    fn split_two(rect: RECT, gap: i32) -> [RECT; 2] {
        let width = (rect_width(rect) - gap) / 2;
        [
            RECT {
                left: rect.left,
                top: rect.top,
                right: rect.left + width,
                bottom: rect.bottom,
            },
            RECT {
                left: rect.left + width + gap,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            },
        ]
    }

    fn split_three(rect: RECT, gap: i32) -> [RECT; 3] {
        let width = (rect_width(rect) - gap * 2) / 3;
        [
            RECT {
                left: rect.left,
                top: rect.top,
                right: rect.left + width,
                bottom: rect.bottom,
            },
            RECT {
                left: rect.left + width + gap,
                top: rect.top,
                right: rect.left + width * 2 + gap,
                bottom: rect.bottom,
            },
            RECT {
                left: rect.left + width * 2 + gap * 2,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            },
        ]
    }

    fn make_rect(x: i32, y: i32, width: i32, height: i32) -> RECT {
        RECT {
            left: x,
            top: y,
            right: x + width.max(0),
            bottom: y + height.max(0),
        }
    }

    fn rect_width(rect: RECT) -> i32 {
        rect.right - rect.left
    }

    fn rect_height(rect: RECT) -> i32 {
        rect.bottom - rect.top
    }

    const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
        COLORREF(red as u32 | ((green as u32) << 8) | ((blue as u32) << 16))
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
            None,
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

    fn utf16(value: &str) -> Vec<u16> {
        value.encode_utf16().collect()
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

    unsafe fn post_boxed_message<T>(hwnd: HWND, message: u32, payload: Box<T>) {
        let payload_ptr = Box::into_raw(payload);
        if PostMessageW(Some(hwnd), message, WPARAM(0), LPARAM(payload_ptr as isize)).is_err() {
            drop(Box::from_raw(payload_ptr));
        }
    }

    unsafe fn drain_worker_messages(hwnd: HWND) {
        let mut message = MSG::default();
        while PeekMessageW(
            &mut message,
            Some(hwnd),
            WM_DRIVECK_PROGRESS,
            WM_DRIVECK_FINISHED,
            PM_REMOVE,
        )
        .into()
        {
            match message.message {
                WM_DRIVECK_PROGRESS => {
                    drop(Box::from_raw(message.lParam.0 as *mut ProgressPayload))
                }
                WM_DRIVECK_FINISHED => {
                    drop(Box::from_raw(message.lParam.0 as *mut FinishedPayload))
                }
                _ => {}
            }
        }
    }
}

#[cfg(windows)]
fn main() {
    app::run();
}
