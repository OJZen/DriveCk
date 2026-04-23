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
    time::{Duration, Instant},
};

use chrono::{Local, TimeZone};
use driveck_ffi::{
    driveck_ffi_format_report_text_json, driveck_ffi_free_string, driveck_ffi_inspect_target_json,
    driveck_ffi_list_targets_json, driveck_ffi_unmount_target_json,
    driveck_ffi_validate_target_json,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use windows::{
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{
            BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush,
            DRAW_TEXT_FORMAT, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE,
            DT_VCENTER, DT_WORDBREAK, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect,
            FrameRect, HDC, HGDIOBJ, IntersectClipRect, InvalidateRect, PAINTSTRUCT, RestoreDC,
            SRCCOPY, SaveDC, ScreenToClient, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
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
            HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
            Input::KeyboardAndMouse::EnableWindow,
            WindowsAndMessaging::{
                BS_PUSHBUTTON, CB_ADDSTRING, CB_DELETESTRING, CB_GETCURSEL, CB_INSERTSTRING,
                CB_RESETCONTENT, CB_SETCURSEL, CBN_SELCHANGE, CBS_DROPDOWNLIST, CREATESTRUCTW,
                CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
                ES_AUTOVSCROLL, ES_LEFT, ES_MULTILINE, ES_READONLY, ES_WANTRETURN, GWLP_USERDATA,
                GetMessageW, GetScrollInfo, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect,
                HMENU, IDC_ARROW, IDOK, KillTimer, LoadCursorW, MB_ICONERROR, MB_ICONWARNING,
                MB_OK, MB_OKCANCEL, MESSAGEBOX_STYLE, MINMAXINFO, MSG, MessageBoxW, MoveWindow,
                PM_REMOVE, PeekMessageW, PostMessageW, PostQuitMessage, RegisterClassW, SB_BOTTOM,
                SB_CTL, SB_LINEDOWN, SB_LINEUP, SB_PAGEDOWN, SB_PAGEUP, SB_THUMBPOSITION,
                SB_THUMBTRACK, SB_TOP, SBM_SETSCROLLINFO, SBS_VERT, SCROLLINFO, SIF_ALL, SIF_PAGE,
                SIF_POS, SIF_RANGE, SM_CXSCREEN, SM_CYSCREEN, SW_HIDE, SW_SHOW, SW_SHOWNORMAL,
                SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetForegroundWindow, SetTimer,
                SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage,
                WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CLOSE, WM_COMMAND, WM_COPY, WM_CREATE,
                WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_MOUSEWHEEL,
                WM_NCCREATE, WM_PAINT, WM_SETFONT, WM_SIZE, WM_TIMER, WM_VSCROLL, WNDCLASSW,
                WS_BORDER, WS_CHILD, WS_CLIPCHILDREN, WS_EX_CLIENTEDGE, WS_OVERLAPPEDWINDOW,
                WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
            },
        },
    },
    core::{PCWSTR, PWSTR},
};

mod about_window;
mod layout;
mod report_window;

use self::layout::{
    Layout, apply_suggested_dpi_rect, current_client_rect, current_layout, inset_rect,
    load_system_ui_font, make_rect, point_in_rect, rect_height, rect_width, redraw_window_now,
    scale_for_window, split_four,
};
use self::{
    about_window::{about_window_proc, open_about_window},
    report_window::{open_report_window, report_window_proc, sync_report_window_from_main_state},
};

const APP_TITLE: &str = concat!("Driveck - v", env!("CARGO_PKG_VERSION"));
const MAIN_CLASS_NAME: &str = "DriveCkWin32Main";
const REPORT_CLASS_NAME: &str = "DriveCkWin32Report";
const ABOUT_CLASS_NAME: &str = "DriveCkWin32About";
const APP_REPOSITORY_URL: &str = "https://github.com/OJZen/DriveCk";
const LABEL_REFRESH: &str = "↻ Refresh";
const LABEL_VALIDATE: &str = "▶ Validate";
const LABEL_STOP: &str = "■ Stop";
const LABEL_SAVE_REPORT: &str = "Save report";
const LABEL_OPEN_REPORT: &str = "Open report";
const LABEL_ABOUT: &str = "ⓘ About";
const LABEL_PANEL_MAP: &str = "Validation Map";
const LABEL_PANEL_REPORT: &str = "Report";
const LABEL_IDLE_BANNER: &str = "Not validated yet";
const LABEL_LIVE_BANNER: &str = "Validation in progress";
const LABEL_REPORT_COPY: &str = "Copy";
const LABEL_REPORT_SAVE: &str = "Save...";
const LABEL_CLOSE: &str = "Close";
const LABEL_GITHUB: &str = "Open GitHub";
const LABEL_ABOUT_TITLE: &str = "DriveCk";
const LABEL_REPORT_PREVIEW: &str = "Report preview";

const IDC_DEVICE_COMBO: i32 = 100;
const IDC_REFRESH: i32 = 101;
const IDC_VALIDATE: i32 = 102;
const IDC_STOP: i32 = 103;
const IDC_SAVE: i32 = 104;
const IDC_PROGRESS: i32 = 105;
const IDC_OPEN_REPORT: i32 = 106;
const IDC_ABOUT: i32 = 107;
const IDC_REPORT_SCROLL: i32 = 108;

const IDC_REPORT_EDIT: i32 = 200;
const IDC_REPORT_COPY: i32 = 201;
const IDC_REPORT_SAVE: i32 = 202;
const IDC_REPORT_CLOSE: i32 = 203;

const IDC_ABOUT_OPEN_GITHUB: i32 = 300;
const IDC_ABOUT_CLOSE: i32 = 301;

const WM_DRIVECK_PROGRESS: u32 = WM_APP + 1;
const WM_DRIVECK_FINISHED: u32 = WM_APP + 2;
const CB_SETMINVISIBLE: u32 = 0x1701;
const EM_SETSEL: u32 = 0x00B1;
const UI_TIMER_ID: usize = 1;

const GRID_ROWS: usize = 18;
const GRID_COLUMNS: usize = 32;
const GRID_SAMPLES: usize = GRID_ROWS * GRID_COLUMNS;

const MIN_WINDOW_WIDTH: i32 = 1000;
const MIN_WINDOW_HEIGHT: i32 = 660;
const MIN_REPORT_WINDOW_WIDTH: i32 = 740;
const MIN_REPORT_WINDOW_HEIGHT: i32 = 560;
const MIN_ABOUT_WINDOW_WIDTH: i32 = 480;
const MIN_ABOUT_WINDOW_HEIGHT: i32 = 320;

const APP_BG: COLORREF = rgb(247, 249, 252);
const SURFACE_BG: COLORREF = rgb(255, 255, 255);
const PANEL_BORDER: COLORREF = rgb(220, 226, 234);
const DIVIDER: COLORREF = rgb(230, 235, 241);
const TEXT_PRIMARY: COLORREF = rgb(28, 35, 43);
const TEXT_MUTED: COLORREF = rgb(96, 108, 123);

const ACCENT_BG: COLORREF = rgb(230, 239, 255);
const ACCENT_FG: COLORREF = rgb(24, 98, 221);
const SUCCESS_BG: COLORREF = rgb(233, 247, 236);
const SUCCESS_FG: COLORREF = rgb(32, 133, 63);
const WARNING_BG: COLORREF = rgb(255, 245, 229);
const WARNING_FG: COLORREF = rgb(185, 102, 0);
const DANGER_BG: COLORREF = rgb(253, 235, 235);
const DANGER_FG: COLORREF = rgb(196, 49, 49);
const NEUTRAL_BG: COLORREF = rgb(240, 243, 247);
const NEUTRAL_FG: COLORREF = rgb(88, 100, 116);

const MAP_PENDING: COLORREF = rgb(219, 223, 228);
const MAP_OK: COLORREF = rgb(101, 184, 59);
const MAP_INVALID: COLORREF = rgb(243, 60, 49);
const MAP_IO: COLORREF = rgb(255, 168, 35);
const MAP_HIGHLIGHT: COLORREF = rgb(34, 122, 255);
const MAP_SURFACE: COLORREF = rgb(252, 253, 255);

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Neutral,
    Accent,
    Success,
    Warning,
    Danger,
}

#[derive(Default, Clone, Copy)]
struct GridCounts {
    processed: usize,
    ok: usize,
    io_errors: usize,
    invalid: usize,
    untested: usize,
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

    fn counts(&self) -> GridCounts {
        counts_from_statuses(&self.sample_status)
    }
}

struct AppState {
    hwnd: HWND,
    device_combo: HWND,
    refresh_button: HWND,
    validate_button: HWND,
    stop_button: HWND,
    save_button: HWND,
    open_report_button: HWND,
    report_scrollbar: HWND,
    about_button: HWND,
    progress_bar: HWND,
    ui_font: HGDIOBJ,
    mono_font: HGDIOBJ,
    owns_ui_font: bool,
    device_targets: Vec<TargetInfo>,
    validation_grid_state: ValidationGridState,
    report_text: Option<String>,
    last_response: Option<ValidationResponse>,
    last_report_target_path: Option<String>,
    worker: Option<JoinHandle<()>>,
    cancel_requested: Arc<AtomicBool>,
    stop_requested: bool,
    closing_requested: bool,
    status_text: String,
    status_tone: Tone,
    report_scroll_offset: i32,
    current_phase: String,
    progress_current: usize,
    progress_total: usize,
    validation_started_at: Option<Instant>,
    validation_started_label: Option<String>,
    last_elapsed_text: Option<String>,
    report_window: Option<HWND>,
    about_window: Option<HWND>,
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
    sample_index: Option<usize>,
    sample_status: Option<SampleStatus>,
}

struct FinishedPayload {
    target_path: String,
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
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let hinstance = HINSTANCE(GetModuleHandleW(None).unwrap().0);

        let icc = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_PROGRESS_CLASS,
        };
        let _ = InitCommonControlsEx(&icc);

        register_window_class(hinstance, MAIN_CLASS_NAME, window_proc);
        register_window_class(hinstance, REPORT_CLASS_NAME, report_window_proc);
        register_window_class(hinstance, ABOUT_CLASS_NAME, about_window_proc);

        let title = wide(APP_TITLE);
        let class_name = wide(MAIN_CLASS_NAME);
        let window_width = 1140;
        let window_height = 740;
        let screen_width = GetSystemMetrics(SM_CXSCREEN);
        let screen_height = GetSystemMetrics(SM_CYSCREEN);
        let window_x = ((screen_width - window_width) / 2).max(0);
        let window_y = ((screen_height - window_height) / 2).max(0);
        let hwnd = CreateWindowExW(
            Default::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN,
            window_x,
            window_y,
            window_width,
            window_height,
            None,
            None,
            Some(hinstance),
            None,
        )
        .expect("create main window");
        set_text(hwnd, APP_TITLE);
        center_window(hwnd, None, window_width, window_height);

        let _ = ShowWindow(hwnd, SW_SHOW);

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
                redraw_window_now(hwnd);
            }
            LRESULT(0)
        }
        WM_VSCROLL => {
            if let Some(state) = state_mut(hwnd) {
                if handle_report_scroll(state, wparam, lparam) {
                    return LRESULT(0);
                }
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_MOUSEWHEEL => {
            if let Some(state) = state_mut(hwnd) {
                if handle_report_mouse_wheel(state, wparam, lparam) {
                    return LRESULT(0);
                }
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_TIMER => {
            if wparam.0 == UI_TIMER_ID {
                if let Some(state) = state_mut(hwnd) {
                    if state.is_busy() {
                        let _ = InvalidateRect(Some(hwnd), None, true);
                    }
                }
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            apply_suggested_dpi_rect(hwnd, lparam);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(lparam.0 as *mut MINMAXINFO);
            info.ptMinTrackSize.x = scale_for_window(hwnd, MIN_WINDOW_WIDTH);
            info.ptMinTrackSize.y = scale_for_window(hwnd, MIN_WINDOW_HEIGHT);
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

                state.progress_current = payload.current;
                state.progress_total = payload.total.max(GRID_SAMPLES);
                state.current_phase = title_case_phrase(&payload.phase);

                let basis_points =
                    progress_basis_points(state.progress_current, state.progress_total);
                send_message(state.progress_bar, PBM_SETPOS, basis_points as isize, 0);

                if state.stop_requested {
                    state.set_status("Stopping", Tone::Warning);
                } else {
                    state.set_status("Validating", Tone::Accent);
                }
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

                let _ = KillTimer(Some(hwnd), UI_TIMER_ID);
                state.cancel_requested.store(false, Ordering::Relaxed);
                state.stop_requested = false;
                state.last_report_target_path = payload
                    .report_text
                    .as_ref()
                    .map(|_| payload.target_path.clone());
                state.last_response = payload.response.clone();
                state.report_text = payload.report_text.clone();
                state.validation_started_at = None;

                if let Some(response) = payload.response.as_ref() {
                    state
                        .validation_grid_state
                        .sync_from_report(&response.report);
                    state.progress_current = response.report.completed_samples;
                    state.progress_total = GRID_SAMPLES;
                    state.validation_started_label =
                        Some(format_local_timestamp(response.report.started_at));
                    state.last_elapsed_text = Some(format_report_elapsed(&response.report));
                    state.current_phase = if response.report.cancelled {
                        "Cancelled".to_string()
                    } else if payload.error.is_some() || report_issue_count(&response.report) != 0 {
                        "Finished with issues".to_string()
                    } else {
                        "Finished".to_string()
                    };

                    let basis_points =
                        progress_basis_points(response.report.completed_samples, GRID_SAMPLES);
                    send_message(state.progress_bar, PBM_SETPOS, basis_points as isize, 0);
                    let (status_text, tone) =
                        final_status_text(payload.error.as_deref(), &response.report);
                    state.set_status(status_text, tone);
                } else {
                    state.validation_started_label = None;
                    state.last_elapsed_text = state
                        .last_elapsed_text
                        .take()
                        .or_else(|| Some("00:00:00".to_string()));
                    state.current_phase = "Failed".to_string();
                    if let Some(error) = payload.error.as_deref() {
                        state.set_status("Failed", Tone::Danger);
                        show_message(hwnd, "Validation failed.", error, MB_ICONERROR);
                    } else {
                        state.set_status("Finished", Tone::Neutral);
                    }
                }

                update_actions(state);
                sync_report_window_from_main_state(state);
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
                    request_stop(state, "Stopping");
                    return LRESULT(0);
                }
            }
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Some(state_ptr) = take_state(hwnd) {
                let mut state = Box::from_raw(state_ptr);
                let _ = KillTimer(Some(hwnd), UI_TIMER_ID);
                if let Some(report_window) = state.report_window.take() {
                    let _ = DestroyWindow(report_window);
                }
                if let Some(about_window) = state.about_window.take() {
                    let _ = DestroyWindow(about_window);
                }
                if state.owns_ui_font {
                    let _ = DeleteObject(state.ui_font);
                }
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
    let (ui_font, owns_ui_font) = load_system_ui_font();
    let mono_font = ui_font;

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
        LABEL_REFRESH,
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
        LABEL_VALIDATE,
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
        LABEL_STOP,
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
        LABEL_SAVE_REPORT,
        hwnd,
        0,
        0,
        0,
        0,
        IDC_SAVE,
        ws(BS_PUSHBUTTON),
    );
    let open_report_button = create_control(
        "BUTTON",
        LABEL_OPEN_REPORT,
        hwnd,
        0,
        0,
        0,
        0,
        IDC_OPEN_REPORT,
        ws(BS_PUSHBUTTON),
    );
    let report_scrollbar = create_control(
        "SCROLLBAR",
        "",
        hwnd,
        0,
        0,
        0,
        0,
        IDC_REPORT_SCROLL,
        ws(SBS_VERT),
    );
    let about_button = create_control(
        "BUTTON",
        LABEL_ABOUT,
        hwnd,
        0,
        0,
        0,
        0,
        IDC_ABOUT,
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

    let state = Box::new(AppState {
        hwnd,
        device_combo,
        refresh_button,
        validate_button,
        stop_button,
        save_button,
        open_report_button,
        report_scrollbar,
        about_button,
        progress_bar,
        ui_font,
        mono_font,
        owns_ui_font,
        device_targets: Vec::new(),
        validation_grid_state: ValidationGridState::default(),
        report_text: None,
        last_response: None,
        last_report_target_path: None,
        worker: None,
        cancel_requested: Arc::new(AtomicBool::new(false)),
        stop_requested: false,
        closing_requested: false,
        status_text: "Select device".to_string(),
        status_tone: Tone::Neutral,
        report_scroll_offset: 0,
        current_phase: "Waiting to start".to_string(),
        progress_current: 0,
        progress_total: GRID_SAMPLES,
        validation_started_at: None,
        validation_started_label: None,
        last_elapsed_text: Some("00:00:00".to_string()),
        report_window: None,
        about_window: None,
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
    for hwnd in [
        state.device_combo,
        state.refresh_button,
        state.validate_button,
        state.stop_button,
        state.save_button,
        state.open_report_button,
        state.about_button,
    ] {
        let _ = SendMessageW(
            hwnd,
            WM_SETFONT,
            Some(WPARAM(state.ui_font.0 as usize)),
            Some(LPARAM(1)),
        );
    }
}

unsafe fn apply_visual_theme(state: &AppState) {
    let theme = wide("Explorer");
    for hwnd in [
        state.device_combo,
        state.refresh_button,
        state.validate_button,
        state.stop_button,
        state.save_button,
        state.open_report_button,
        state.about_button,
        state.progress_bar,
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
        IDC_STOP => request_stop(state, "Stopping"),
        IDC_SAVE => save_current_report(state),
        IDC_OPEN_REPORT => open_report_window(state),
        IDC_ABOUT => open_about_window(state),
        IDC_DEVICE_COMBO if code as u32 == CBN_SELCHANGE => {
            clear_run_output_for_new_selection(state);
            update_idle_status(state);
            update_actions(state);
            sync_report_window_from_main_state(state);
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
        _ => {}
    }
}

unsafe fn start_validation(state: &mut AppState) {
    let path = match state.selected_target() {
        Some(target) => target.path.clone(),
        None => {
            let error = "Choose a removable or USB whole-disk device first.".to_string();
            state.set_status("Start failed", Tone::Danger);
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
            state.set_status("Start failed", Tone::Danger);
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

    let detail = build_validation_confirmation(&target);
    let detail_text = wide(&detail);
    let title = wide("Validate selected device?");
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
            state.set_status("Start failed", Tone::Danger);
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
    state.last_report_target_path = None;
    state.validation_grid_state.reset();
    state.current_phase = "Validating".to_string();
    state.progress_current = 0;
    state.progress_total = GRID_SAMPLES;
    state.validation_started_at = Some(Instant::now());
    state.validation_started_label = Some(Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
    state.last_elapsed_text = Some("00:00:00".to_string());
    state.set_status("Validating", Tone::Accent);
    send_message(state.progress_bar, PBM_SETPOS, 0, 0);
    let _ = SetTimer(Some(state.hwnd), UI_TIMER_ID, 1000, None);

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
    update_actions(state);
    sync_report_window_from_main_state(state);
    let _ = InvalidateRect(Some(state.hwnd), None, true);
}

unsafe fn request_stop(state: &mut AppState, status_text: &str) {
    if state.worker.is_none() || state.stop_requested {
        return;
    }
    state.cancel_requested.store(true, Ordering::Relaxed);
    state.stop_requested = true;
    state.current_phase = "Stopping".to_string();
    state.set_status(status_text, Tone::Warning);
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
                target_path: target.path.clone(),
                response: execution.response,
                report_text,
                error,
            }
        }
        Err(error) => FinishedPayload {
            target_path: target.path.clone(),
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
        state.current_phase = "Unmounting".to_string();
        state.set_status("Unmounting", Tone::Warning);
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
            state.set_status("Refresh failed", Tone::Danger);
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

    if state.last_report_target_path.as_ref().is_some_and(|path| {
        Some(path.as_str()) != state.selected_target().map(|target| target.path.as_str())
    }) {
        clear_run_output(state);
    }

    update_idle_status(state);
    update_actions(state);
    sync_report_window_from_main_state(state);
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

    match save_report_text(state.hwnd, report_text) {
        Ok(()) => state.set_status("Saved", Tone::Success),
        Err(error) => {
            state.set_status("Save failed", Tone::Danger);
            show_message(state.hwnd, "Failed to save report.", &error, MB_ICONERROR);
        }
    }
    let _ = InvalidateRect(Some(state.hwnd), None, true);
}

unsafe fn update_actions(state: &mut AppState) {
    let busy = state.is_busy();
    let report_ready = !busy && state.report_text.is_some();
    let can_validate = state.selected_target().is_some() && !busy;

    enable(
        state.device_combo,
        !busy && !state.device_targets.is_empty(),
    );
    enable(state.refresh_button, !busy);
    enable(state.validate_button, can_validate);
    enable(state.stop_button, busy && !state.stop_requested);
    enable(state.save_button, report_ready);
    enable(state.open_report_button, report_ready);
    enable(state.about_button, true);
    let _ = ShowWindow(state.validate_button, if busy { SW_HIDE } else { SW_SHOW });
    let _ = ShowWindow(state.stop_button, if busy { SW_SHOW } else { SW_HIDE });
    sync_report_scrollbar(state);
}

unsafe fn clear_run_output_for_new_selection(state: &mut AppState) {
    if state.is_busy() {
        return;
    }
    if state.report_text.is_some() || state.last_response.is_some() {
        clear_run_output(state);
    }
}

unsafe fn clear_run_output(state: &mut AppState) {
    state.validation_grid_state.reset();
    state.report_text = None;
    state.last_response = None;
    state.last_report_target_path = None;
    state.current_phase = "Waiting to start".to_string();
    state.progress_current = 0;
    state.progress_total = GRID_SAMPLES;
    state.validation_started_at = None;
    state.validation_started_label = None;
    state.last_elapsed_text = Some("00:00:00".to_string());
    send_message(state.progress_bar, PBM_SETPOS, 0, 0);
}

unsafe fn update_idle_status(state: &mut AppState) {
    if state.is_busy() || state.last_response.is_some() {
        return;
    }
    match state.selected_target() {
        Some(target) if target.is_mounted => state.set_status("Will dismount", Tone::Warning),
        Some(_) => state.set_status("Ready", Tone::Success),
        None if state.device_targets.is_empty() => state.set_status("No device", Tone::Neutral),
        None => state.set_status("Select device", Tone::Neutral),
    }
}

unsafe fn layout_child_controls(state: &mut AppState) {
    let layout = current_layout(state.hwnd);
    for (hwnd, rect) in [
        (state.device_combo, layout.combo),
        (state.refresh_button, layout.refresh_button),
        (state.validate_button, layout.validate_button),
        (state.stop_button, layout.stop_button),
        (state.save_button, layout.save_button),
        (state.about_button, layout.about_button),
        (state.open_report_button, layout.report_button),
        (state.report_scrollbar, layout.report_scrollbar),
        (state.progress_bar, layout.progress),
    ] {
        let _ = MoveWindow(
            hwnd,
            rect.left,
            rect.top,
            rect_width(rect),
            rect_height(rect),
            true,
        );
    }
    sync_report_scrollbar(state);
}

fn report_content_height(state: &AppState) -> i32 {
    if state.last_response.is_some() {
        scale_for_window(state.hwnd, 560)
    } else if state.is_busy() {
        scale_for_window(state.hwnd, 320)
    } else {
        scale_for_window(state.hwnd, 260)
    }
}

unsafe fn sync_report_scrollbar(state: &mut AppState) {
    let layout = current_layout(state.hwnd);
    let viewport_height = rect_height(layout.report_scrollbar).max(0);
    let content_height = report_content_height(state).max(viewport_height);
    let max_offset = (content_height - viewport_height).max(0);
    state.report_scroll_offset = state.report_scroll_offset.clamp(0, max_offset);

    let mut info = SCROLLINFO::default();
    info.cbSize = size_of::<SCROLLINFO>() as u32;
    info.fMask = SIF_RANGE | SIF_PAGE | SIF_POS;
    info.nMin = 0;
    info.nMax = content_height.saturating_sub(1);
    info.nPage = viewport_height.max(1) as u32;
    info.nPos = state.report_scroll_offset;
    let _ = send_message(
        state.report_scrollbar,
        SBM_SETSCROLLINFO,
        1,
        &info as *const SCROLLINFO as isize,
    );
    let _ = ShowWindow(
        state.report_scrollbar,
        if max_offset > 0 { SW_SHOW } else { SW_HIDE },
    );
}

unsafe fn handle_report_scroll(state: &mut AppState, wparam: WPARAM, lparam: LPARAM) -> bool {
    if HWND(lparam.0 as *mut c_void) != state.report_scrollbar {
        return false;
    }

    let mut info = SCROLLINFO::default();
    info.cbSize = size_of::<SCROLLINFO>() as u32;
    info.fMask = SIF_ALL;
    let _ = GetScrollInfo(state.report_scrollbar, SB_CTL, &mut info);

    let code = (wparam.0 & 0xffff) as i32;
    let mut position = info.nPos;
    match code {
        x if x == SB_LINEUP.0 => position -= scale_for_window(state.hwnd, 24),
        x if x == SB_LINEDOWN.0 => position += scale_for_window(state.hwnd, 24),
        x if x == SB_PAGEUP.0 => position -= info.nPage as i32,
        x if x == SB_PAGEDOWN.0 => position += info.nPage as i32,
        x if x == SB_THUMBPOSITION.0 || x == SB_THUMBTRACK.0 => position = info.nTrackPos,
        x if x == SB_TOP.0 => position = 0,
        x if x == SB_BOTTOM.0 => position = info.nMax - info.nPage as i32 + 1,
        _ => {}
    }

    let max_offset = (info.nMax - info.nPage as i32 + 1).max(0);
    position = position.clamp(0, max_offset);
    if position != state.report_scroll_offset {
        state.report_scroll_offset = position;
        info.fMask = SIF_POS;
        info.nPos = position;
        let _ = send_message(
            state.report_scrollbar,
            SBM_SETSCROLLINFO,
            1,
            &info as *const SCROLLINFO as isize,
        );
        let _ = InvalidateRect(Some(state.hwnd), None, true);
    }
    true
}

unsafe fn handle_report_mouse_wheel(state: &mut AppState, wparam: WPARAM, lparam: LPARAM) -> bool {
    let layout = current_layout(state.hwnd);
    let mut point = POINT {
        x: (lparam.0 as u32 & 0xffff) as i16 as i32,
        y: ((lparam.0 as u32 >> 16) & 0xffff) as i16 as i32,
    };
    let _ = ScreenToClient(state.hwnd, &mut point);
    if !point_in_rect(layout.report_panel, point.x, point.y) {
        return false;
    }

    let mut info = SCROLLINFO::default();
    info.cbSize = size_of::<SCROLLINFO>() as u32;
    info.fMask = SIF_ALL;
    let _ = GetScrollInfo(state.report_scrollbar, SB_CTL, &mut info);

    let wheel_delta = (((wparam.0 >> 16) & 0xffff) as i16) as i32;
    if wheel_delta == 0 {
        return false;
    }

    let step = scale_for_window(state.hwnd, 72);
    let mut position = info.nPos - (wheel_delta / 120) * step;
    let max_offset = (info.nMax - info.nPage as i32 + 1).max(0);
    position = position.clamp(0, max_offset);
    if position != state.report_scroll_offset {
        state.report_scroll_offset = position;
        info.fMask = SIF_POS;
        info.nPos = position;
        let _ = send_message(
            state.report_scrollbar,
            SBM_SETSCROLLINFO,
            1,
            &info as *const SCROLLINFO as isize,
        );
        let _ = InvalidateRect(Some(state.hwnd), None, true);
    }
    true
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

fn save_report_text(hwnd: HWND, report_text: &str) -> Result<(), String> {
    let Some(path) = (unsafe { pick_save_path() }) else {
        return Ok(());
    };

    fs::write(&path, report_text)
        .map_err(|error| format!("Failed to write report {}: {}", path, error))?;

    let _ = hwnd;
    Ok(())
}

unsafe fn center_window(hwnd: HWND, parent: Option<HWND>, width: i32, height: i32) {
    let (origin_x, origin_y, area_width, area_height) = if let Some(parent) = parent {
        let mut rect = RECT::default();
        let _ = GetWindowRect(parent, &mut rect);
        (rect.left, rect.top, rect_width(rect), rect_height(rect))
    } else {
        (
            0,
            0,
            GetSystemMetrics(SM_CXSCREEN),
            GetSystemMetrics(SM_CYSCREEN),
        )
    };
    let x = origin_x + ((area_width - width) / 2).max(0);
    let y = origin_y + ((area_height - height) / 2).max(0);
    let _ = SetWindowPos(
        hwnd,
        None,
        x,
        y,
        width,
        height,
        SWP_NOZORDER | SWP_NOACTIVATE,
    );
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

    paint_main_scene(back_dc, state, &layout);
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

unsafe fn paint_main_scene(hdc: HDC, state: &AppState, layout: &Layout) {
    let client = current_client_rect(state.hwnd);
    fill_rect_color(hdc, &client, APP_BG);

    draw_panel(hdc, &layout.header_panel);
    draw_panel(hdc, &layout.map_panel);
    draw_panel(hdc, &layout.report_panel);
    draw_panel(hdc, &layout.footer_panel);

    paint_header_panel(hdc, state, layout);
    paint_map_panel(hdc, state, &layout.map_panel);
    paint_report_panel(hdc, state, &layout.report_panel);
    paint_footer(hdc, state, layout);
}

unsafe fn paint_header_panel(hdc: HDC, state: &AppState, layout: &Layout) {
    let info_top = layout.combo.bottom + scale_for_window(state.hwnd, 10);
    let info_rect = make_rect(
        layout.header_panel.left + scale_for_window(state.hwnd, 14),
        info_top,
        rect_width(layout.header_panel) - scale_for_window(state.hwnd, 28),
        (layout.header_panel.bottom - info_top - scale_for_window(state.hwnd, 12)).max(0),
    );

    if let Some(target) = state.selected_target() {
        let sections = split_four(info_rect, scale_for_window(state.hwnd, 10));
        draw_header_metric(
            hdc,
            sections[0],
            "Status",
            &device_status_label(state, target),
            tone_text_color(device_status_tone(state, target)),
            state.ui_font,
        );
        draw_header_metric(
            hdc,
            sections[1],
            "Model",
            &device_display_name(target),
            TEXT_PRIMARY,
            state.ui_font,
        );
        draw_header_metric(
            hdc,
            sections[2],
            "Capacity",
            &format_bytes(target.size_bytes),
            TEXT_PRIMARY,
            state.ui_font,
        );
        draw_header_metric(
            hdc,
            sections[3],
            "Interface",
            &device_transport_text(target),
            TEXT_PRIMARY,
            state.ui_font,
        );
    } else {
        draw_text_block(
            hdc,
            info_rect,
            "Select a removable or USB whole-disk target.",
            TEXT_MUTED,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
            state.ui_font,
        );
    }
}

#[allow(dead_code)]
unsafe fn paint_device_panel(hdc: HDC, state: &AppState, panel: &RECT) {
    let row_height = scale_for_window(state.hwnd, 22);
    let row_gap = scale_for_window(state.hwnd, 28);
    let content = draw_panel_header(hdc, panel, "Device", None, state.ui_font);
    if let Some(target) = state.selected_target() {
        let mut row_top = content.top;
        draw_key_value_row_tone(
            hdc,
            make_rect(content.left, row_top, rect_width(content), row_height),
            "Status",
            &device_status_label(state, target),
            device_status_tone(state, target),
            state.ui_font,
        );
        row_top += row_gap;
        draw_key_value_row(
            hdc,
            make_rect(content.left, row_top, rect_width(content), row_height),
            "Model",
            &device_display_name(target),
            state.ui_font,
        );
        row_top += row_gap;
        draw_key_value_row(
            hdc,
            make_rect(content.left, row_top, rect_width(content), row_height),
            "Path",
            &target.path,
            state.ui_font,
        );
        row_top += row_gap;
        draw_key_value_row(
            hdc,
            make_rect(content.left, row_top, rect_width(content), row_height),
            "Capacity",
            &format_bytes(target.size_bytes),
            state.ui_font,
        );
        row_top += row_gap;
        draw_key_value_row(
            hdc,
            make_rect(content.left, row_top, rect_width(content), row_height),
            "Interface",
            &device_transport_text(target),
            state.ui_font,
        );
    } else {
        draw_text_block(
            hdc,
            content,
            "No removable or USB whole-disk device is currently available.\r\n\r\nInsert a target or unmount the disk, then click Refresh.",
            TEXT_MUTED,
            DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
            state.ui_font,
        );
    }
}

#[allow(dead_code)]
unsafe fn paint_validation_panel(hdc: HDC, state: &AppState, panel: &RECT) {
    let content = draw_panel_header(hdc, panel, "Validation", None, state.ui_font);
    let row_height = 24;
    let row_gap = 30;
    let mut row_top = content.top;
    let status_text = validation_status_label(state);
    let status_tone = validation_status_tone(state);
    draw_key_value_row_tone(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Status",
        &status_text,
        status_tone,
        state.ui_font,
    );
    row_top += row_gap;

    draw_key_value_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Current stage",
        &state.current_phase,
        state.ui_font,
    );
    row_top += row_gap;

    draw_key_value_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Processed",
        &format!("{} / {}", state.progress_current, state.progress_total),
        state.ui_font,
    );
    row_top += row_gap;

    draw_key_value_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Progress",
        &progress_percent_text(state.progress_current, state.progress_total),
        state.ui_font,
    );
    row_top += row_gap;

    draw_key_value_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Started",
        state.validation_started_label.as_deref().unwrap_or("-"),
        state.ui_font,
    );
    row_top += row_gap;

    draw_key_value_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Elapsed",
        &current_elapsed_text(state),
        state.ui_font,
    );
    row_top += row_gap;

    draw_key_value_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Report",
        if state.report_text.is_some() {
            "Ready"
        } else if state.is_busy() {
            "After completion"
        } else {
            "Unavailable"
        },
        state.ui_font,
    );

    let note_rect = make_rect(
        content.left,
        row_top + 16,
        rect_width(content),
        (content.bottom - row_top - 16).max(0),
    );
    let note = if state.is_busy() {
        "Stop remains available while validation is running."
    } else if state.report_text.is_some() {
        "The detailed shared Rust report is ready to open."
    } else {
        "Validation temporarily writes sampled regions and restores them afterwards."
    };
    draw_text_block(
        hdc,
        note_rect,
        note,
        TEXT_MUTED,
        DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
        state.ui_font,
    );
}

unsafe fn paint_map_panel(hdc: HDC, state: &AppState, panel: &RECT) {
    let content = draw_panel_header(hdc, panel, LABEL_PANEL_MAP, None, state.ui_font);
    let legend_height = 28;
    let map_rect = make_rect(
        content.left,
        content.top,
        rect_width(content),
        rect_height(content) - legend_height - 14,
    );
    let legend_rect = make_rect(
        content.left,
        map_rect.bottom + 14,
        rect_width(content),
        legend_height,
    );

    draw_validation_map(hdc, &map_rect, &state.validation_grid_state);
    draw_map_legend(hdc, legend_rect, state.ui_font);
}

#[allow(dead_code)]
unsafe fn paint_summary_panel(hdc: HDC, state: &AppState, panel: &RECT) {
    let content = draw_panel_header(hdc, panel, "Run Status", None, state.ui_font);
    let row_height = scale_for_window(state.hwnd, 22);
    let row_gap = scale_for_window(state.hwnd, 28);
    let mut row_top = content.top;
    let processed = format!("{} / {}", state.progress_current, state.progress_total);
    let report_state = if state.report_text.is_some() {
        "Ready"
    } else if state.is_busy() {
        "Pending"
    } else {
        "Unavailable"
    };

    draw_key_value_row_tone(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Status",
        &validation_status_label(state),
        validation_status_tone(state),
        state.ui_font,
    );
    row_top += row_gap;
    draw_key_value_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Processed",
        &processed,
        state.ui_font,
    );
    row_top += row_gap;
    draw_key_value_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Progress",
        &progress_percent_text(state.progress_current, state.progress_total),
        state.ui_font,
    );
    row_top += row_gap;
    draw_key_value_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Elapsed",
        &current_elapsed_text(state),
        state.ui_font,
    );
    row_top += row_gap;
    draw_key_value_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Report",
        report_state,
        state.ui_font,
    );
}

unsafe fn paint_idle_summary(hdc: HDC, state: &AppState, content: RECT) {
    let row_height = scale_for_window(state.hwnd, 22);
    let row_gap = scale_for_window(state.hwnd, 28);
    let banner = make_rect(
        content.left,
        content.top,
        rect_width(content),
        scale_for_window(state.hwnd, 44),
    );
    draw_banner(
        hdc,
        &banner,
        LABEL_IDLE_BANNER,
        "Run validation to populate the summary and report.",
        Tone::Accent,
        state.ui_font,
    );

    let mut row_top = banner.bottom + scale_for_window(state.hwnd, 10);
    let target_text = state
        .selected_target()
        .map(device_display_name)
        .unwrap_or_else(|| "-".to_string());
    draw_metric_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Target",
        &target_text,
        state.ui_font,
    );
    row_top += row_gap;
    draw_metric_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Processed",
        "0 of 576 regions",
        state.ui_font,
    );
    row_top += row_gap;
    draw_metric_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Report",
        "Unavailable",
        state.ui_font,
    );
    row_top += row_gap;
    draw_metric_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Elapsed",
        &current_elapsed_text(state),
        state.ui_font,
    );
}

unsafe fn paint_live_summary(hdc: HDC, state: &AppState, content: RECT) {
    let counts = state.validation_grid_state.counts();
    let row_height = scale_for_window(state.hwnd, 22);
    let row_gap = scale_for_window(state.hwnd, 28);
    let banner = make_rect(
        content.left,
        content.top,
        rect_width(content),
        scale_for_window(state.hwnd, 58),
    );
    draw_banner(
        hdc,
        &banner,
        LABEL_LIVE_BANNER,
        &state.current_phase,
        if state.stop_requested {
            Tone::Warning
        } else {
            Tone::Accent
        },
        state.ui_font,
    );

    let mut row_top = banner.bottom + scale_for_window(state.hwnd, 10);
    let processed = format!("{} of {} regions", counts.processed, GRID_SAMPLES);
    draw_metric_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Processed",
        &processed,
        state.ui_font,
    );
    row_top += row_gap;
    draw_metric_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Valid",
        &counts.ok.to_string(),
        state.ui_font,
    );
    row_top += row_gap;
    draw_metric_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Invalid",
        &counts.invalid.to_string(),
        state.ui_font,
    );
    row_top += row_gap;
    draw_metric_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "I/O errors",
        &counts.io_errors.to_string(),
        state.ui_font,
    );
    row_top += row_gap;
    draw_metric_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Unvalidated",
        &counts.untested.to_string(),
        state.ui_font,
    );
    row_top += row_gap + scale_for_window(state.hwnd, 6);
    draw_divider(
        hdc,
        content.left,
        content.right,
        row_top - scale_for_window(state.hwnd, 8),
    );
    draw_metric_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Elapsed time",
        &current_elapsed_text(state),
        state.ui_font,
    );
    row_top += row_gap;
    draw_metric_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Report",
        "Available after completion",
        state.ui_font,
    );
}

unsafe fn paint_result_summary(
    hdc: HDC,
    state: &AppState,
    response: &ValidationResponse,
    content: RECT,
) {
    let report = &response.report;
    let counts = counts_from_statuses(&report.sample_status);
    let tone = report_banner_tone(report);
    let row_height = scale_for_window(state.hwnd, 22);
    let row_gap = scale_for_window(state.hwnd, 28);
    let banner = make_rect(
        content.left,
        content.top,
        rect_width(content),
        scale_for_window(state.hwnd, 64),
    );
    draw_banner(
        hdc,
        &banner,
        report_banner_title(report),
        report_banner_subtitle(report),
        tone,
        state.ui_font,
    );

    let mut row_top = banner.bottom + scale_for_window(state.hwnd, 10);
    let processed = format!("{} of {} regions", report.completed_samples, GRID_SAMPLES);
    draw_metric_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Processed",
        &processed,
        state.ui_font,
    );
    row_top += row_gap;
    draw_metric_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Valid regions",
        &counts.ok.to_string(),
        state.ui_font,
    );
    row_top += row_gap;
    draw_metric_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Invalid regions",
        &counts.invalid.to_string(),
        state.ui_font,
    );
    row_top += row_gap;
    draw_metric_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "I/O error regions",
        &counts.io_errors.to_string(),
        state.ui_font,
    );
    row_top += row_gap;
    draw_metric_row(
        hdc,
        make_rect(content.left, row_top, rect_width(content), row_height),
        "Unvalidated regions",
        &counts.untested.to_string(),
        state.ui_font,
    );
    row_top += row_gap + scale_for_window(state.hwnd, 6);
    draw_divider(
        hdc,
        content.left,
        content.right,
        row_top - scale_for_window(state.hwnd, 8),
    );

    for (label, value) in [
        ("Failures", report_issue_count(report).to_string()),
        ("Reported size", format_bytes(report.reported_size_bytes)),
        (
            "Validated size",
            format_bytes(report.validated_drive_size_bytes),
        ),
        (
            "Highest valid region",
            format_bytes(report.highest_valid_region_bytes),
        ),
        ("Region size", format_bytes(report.region_size_bytes)),
        (
            "Report",
            if state.report_text.is_some() {
                "Ready".to_string()
            } else {
                "Unavailable".to_string()
            },
        ),
    ] {
        draw_metric_row(
            hdc,
            make_rect(content.left, row_top, rect_width(content), row_height),
            label,
            &value,
            state.ui_font,
        );
        row_top += row_gap;
    }

    let failure_detail = format_failure_summary(report);
    draw_metric_multiline(
        hdc,
        make_rect(
            content.left,
            row_top,
            rect_width(content),
            scale_for_window(state.hwnd, 32),
        ),
        "Failure detail",
        &failure_detail,
        state.ui_font,
    );
}

unsafe fn paint_report_panel(hdc: HDC, state: &AppState, panel: &RECT) {
    let content = draw_panel_header(hdc, panel, LABEL_PANEL_REPORT, None, state.ui_font);
    let action_space = scale_for_window(state.hwnd, 52);
    let scrollbar_width = scale_for_window(state.hwnd, 14);
    let viewport = make_rect(
        content.left,
        content.top,
        (rect_width(content) - scrollbar_width - scale_for_window(state.hwnd, 8)).max(0),
        (rect_height(content) - action_space).max(scale_for_window(state.hwnd, 96)),
    );
    let report_content = make_rect(
        viewport.left,
        viewport.top - state.report_scroll_offset,
        rect_width(viewport),
        report_content_height(state),
    );
    let saved = SaveDC(hdc);
    let _ = IntersectClipRect(
        hdc,
        viewport.left,
        viewport.top,
        viewport.right,
        viewport.bottom,
    );
    if let Some(response) = state.last_response.as_ref() {
        paint_result_summary(hdc, state, response, report_content);
    } else if state.is_busy() {
        paint_live_summary(hdc, state, report_content);
    } else {
        paint_idle_summary(hdc, state, report_content);
    }
    let _ = RestoreDC(hdc, saved);
}

unsafe fn paint_footer(hdc: HDC, state: &AppState, layout: &Layout) {
    draw_text_block(
        hdc,
        layout.status_label,
        &footer_stage_text(state),
        tone_text_color(footer_stage_tone(state)),
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        state.ui_font,
    );
    draw_text_block(
        hdc,
        layout.progress_label,
        "Progress",
        TEXT_MUTED,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        state.ui_font,
    );
    draw_text_block(
        hdc,
        layout.percent,
        &progress_percent_text(state.progress_current, state.progress_total),
        TEXT_PRIMARY,
        DT_RIGHT | DT_SINGLELINE | DT_VCENTER,
        state.ui_font,
    );
    draw_text_block(
        hdc,
        layout.elapsed,
        &format!("Elapsed: {}", current_elapsed_text(state)),
        TEXT_MUTED,
        DT_RIGHT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        state.ui_font,
    );
}

unsafe fn draw_validation_map(hdc: HDC, map_bounds: &RECT, grid_state: &ValidationGridState) {
    draw_panel_surface(hdc, map_bounds, MAP_SURFACE);

    let gap = 2;
    let padding = 8;
    let available_width = rect_width(*map_bounds) - gap * (GRID_COLUMNS as i32 - 1) - padding * 2;
    let available_height = rect_height(*map_bounds) - gap * (GRID_ROWS as i32 - 1) - padding * 2;
    let cell_width = (available_width / GRID_COLUMNS as i32).max(2);
    let cell_height = (available_height / GRID_ROWS as i32).max(2);
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
            let color = sample_status_color(
                grid_state
                    .sample_status
                    .get(index)
                    .copied()
                    .unwrap_or(SampleStatus::Untested),
            );
            fill_rect_color(hdc, &cell_rect, color);
            frame_rect_color(hdc, &cell_rect, rgb(255, 255, 255));
            if grid_state.last_sample == Some(index) {
                frame_rect_color(hdc, &cell_rect, MAP_HIGHLIGHT);
            }
        }
    }
}

unsafe fn draw_map_legend(hdc: HDC, rect: RECT, font: HGDIOBJ) {
    let items = split_four(rect, 10);
    draw_legend_item(hdc, items[0], "Valid", MAP_OK, None, font);
    draw_legend_item(hdc, items[1], "Invalid", MAP_INVALID, None, font);
    draw_legend_item(hdc, items[2], "I/O error", MAP_IO, None, font);
    draw_legend_item(hdc, items[3], "Unvalidated", MAP_PENDING, None, font);
}

unsafe fn draw_legend_item(
    hdc: HDC,
    rect: RECT,
    label: &str,
    fill: COLORREF,
    border: Option<COLORREF>,
    font: HGDIOBJ,
) {
    let swatch = make_rect(rect.left, rect.top + 4, 16, 16);
    fill_rect_color(hdc, &swatch, fill);
    frame_rect_color(hdc, &swatch, border.unwrap_or(fill));
    let text_rect = make_rect(
        rect.left + 24,
        rect.top,
        rect_width(rect) - 24,
        rect_height(rect),
    );
    draw_text_block(
        hdc,
        text_rect,
        label,
        TEXT_MUTED,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        font,
    );
}

unsafe fn draw_panel(hdc: HDC, rect: &RECT) {
    fill_rect_color(hdc, rect, SURFACE_BG);
    frame_rect_color(hdc, rect, PANEL_BORDER);
}

unsafe fn draw_panel_surface(hdc: HDC, rect: &RECT, background: COLORREF) {
    fill_rect_color(hdc, rect, background);
    frame_rect_color(hdc, rect, PANEL_BORDER);
}

unsafe fn draw_panel_header(
    hdc: HDC,
    panel: &RECT,
    title: &str,
    trailing: Option<&str>,
    font: HGDIOBJ,
) -> RECT {
    let inner = inset_rect(*panel, 14, 12);
    let title_rect = make_rect(inner.left, inner.top, rect_width(inner), 20);
    draw_text_block(
        hdc,
        title_rect,
        title,
        TEXT_PRIMARY,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        font,
    );
    if let Some(trailing) = trailing {
        draw_text_block(
            hdc,
            title_rect,
            trailing,
            TEXT_MUTED,
            DT_RIGHT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
            font,
        );
    }
    let divider_y = title_rect.bottom + 8;
    draw_divider(hdc, panel.left, panel.right, divider_y);
    make_rect(
        inner.left,
        divider_y + 12,
        rect_width(inner),
        panel.bottom - divider_y - 16,
    )
}

unsafe fn draw_banner(
    hdc: HDC,
    rect: &RECT,
    title: &str,
    subtitle: &str,
    tone: Tone,
    font: HGDIOBJ,
) {
    let (background, foreground) = tone_palette(tone);
    fill_rect_color(hdc, rect, background);
    frame_rect_color(hdc, rect, background);

    let inner = inset_rect(*rect, 14, 10);
    let title_rect = make_rect(inner.left, inner.top, rect_width(inner), 20);
    draw_text_block(
        hdc,
        title_rect,
        title,
        foreground,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        font,
    );
    let subtitle_rect = make_rect(inner.left, title_rect.bottom + 2, rect_width(inner), 18);
    draw_text_block(
        hdc,
        subtitle_rect,
        subtitle,
        foreground,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        font,
    );
}

#[allow(dead_code)]
unsafe fn draw_inline_status(hdc: HDC, rect: RECT, text: &str, tone: Tone, font: HGDIOBJ) {
    let dot_rect = make_rect(rect.left, rect.top + 4, 12, 12);
    fill_rect_color(hdc, &dot_rect, tone_text_color(tone));
    frame_rect_color(hdc, &dot_rect, tone_text_color(tone));
    let text_rect = make_rect(
        rect.left + 20,
        rect.top,
        rect_width(rect) - 20,
        rect_height(rect),
    );
    draw_text_block(
        hdc,
        text_rect,
        text,
        tone_text_color(tone),
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        font,
    );
}

unsafe fn draw_header_metric(
    hdc: HDC,
    rect: RECT,
    label: &str,
    value: &str,
    value_color: COLORREF,
    font: HGDIOBJ,
) {
    let label_height = ((rect_height(rect) * 40) / 100).clamp(12, 16);
    let label_rect = make_rect(rect.left, rect.top, rect_width(rect), label_height);
    draw_text_block(
        hdc,
        label_rect,
        label,
        TEXT_MUTED,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        font,
    );
    let value_rect = make_rect(
        rect.left,
        label_rect.bottom + 2,
        rect_width(rect),
        (rect_height(rect) - rect_height(label_rect) - 2).max(0),
    );
    draw_text_block(
        hdc,
        value_rect,
        value,
        value_color,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        font,
    );
}

unsafe fn draw_key_value_row(hdc: HDC, rect: RECT, key: &str, value: &str, font: HGDIOBJ) {
    draw_key_value_row_with_color(hdc, rect, key, value, TEXT_PRIMARY, font);
}

unsafe fn draw_key_value_row_tone(
    hdc: HDC,
    rect: RECT,
    key: &str,
    value: &str,
    tone: Tone,
    font: HGDIOBJ,
) {
    draw_key_value_row_with_color(hdc, rect, key, value, tone_text_color(tone), font);
}

unsafe fn draw_key_value_row_with_color(
    hdc: HDC,
    rect: RECT,
    key: &str,
    value: &str,
    value_color: COLORREF,
    font: HGDIOBJ,
) {
    let key_width = ((rect_width(rect) * 31) / 100).clamp(92, 126);
    let key_rect = make_rect(rect.left, rect.top, key_width, rect_height(rect));
    let value_rect = make_rect(
        rect.left + key_width + 10,
        rect.top,
        rect_width(rect) - key_width - 10,
        rect_height(rect),
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
        value_color,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        font,
    );
}

unsafe fn draw_metric_row(hdc: HDC, rect: RECT, key: &str, value: &str, font: HGDIOBJ) {
    let key_width = ((rect_width(rect) * 42) / 100).clamp(102, 152);
    let key_rect = make_rect(rect.left, rect.top, key_width, rect_height(rect));
    let value_rect = make_rect(
        rect.left + key_width + 8,
        rect.top,
        rect_width(rect) - key_width - 8,
        rect_height(rect),
    );
    draw_text_block(
        hdc,
        key_rect,
        key,
        TEXT_PRIMARY,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        font,
    );
    draw_text_block(
        hdc,
        value_rect,
        value,
        TEXT_MUTED,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        font,
    );
}

unsafe fn draw_metric_multiline(hdc: HDC, rect: RECT, key: &str, value: &str, font: HGDIOBJ) {
    let key_width = ((rect_width(rect) * 36) / 100).clamp(100, 138);
    let key_rect = make_rect(rect.left, rect.top, key_width, rect_height(rect));
    let value_rect = make_rect(
        rect.left + key_width + 8,
        rect.top,
        rect_width(rect) - key_width - 8,
        rect_height(rect),
    );
    draw_text_block(
        hdc,
        key_rect,
        key,
        TEXT_PRIMARY,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        font,
    );
    draw_text_block(
        hdc,
        value_rect,
        value,
        TEXT_MUTED,
        DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
        font,
    );
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

unsafe fn draw_divider(hdc: HDC, left: i32, right: i32, y: i32) {
    let rect = make_rect(left + 1, y, (right - left - 2).max(0), 1);
    fill_rect_color(hdc, &rect, DIVIDER);
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
        Tone::Neutral => (NEUTRAL_BG, NEUTRAL_FG),
        Tone::Accent => (ACCENT_BG, ACCENT_FG),
        Tone::Success => (SUCCESS_BG, SUCCESS_FG),
        Tone::Warning => (WARNING_BG, WARNING_FG),
        Tone::Danger => (DANGER_BG, DANGER_FG),
    }
}

fn tone_text_color(tone: Tone) -> COLORREF {
    tone_palette(tone).1
}

fn sample_status_color(status: SampleStatus) -> COLORREF {
    match status {
        SampleStatus::Untested => MAP_PENDING,
        SampleStatus::Ok => MAP_OK,
        SampleStatus::ReadError | SampleStatus::WriteError => MAP_IO,
        SampleStatus::VerifyMismatch | SampleStatus::RestoreError => MAP_INVALID,
    }
}

fn counts_from_statuses(statuses: &[SampleStatus]) -> GridCounts {
    let mut counts = GridCounts::default();
    for status in statuses.iter().copied().take(GRID_SAMPLES) {
        match status {
            SampleStatus::Untested => counts.untested += 1,
            SampleStatus::Ok => {
                counts.ok += 1;
                counts.processed += 1;
            }
            SampleStatus::ReadError | SampleStatus::WriteError => {
                counts.io_errors += 1;
                counts.processed += 1;
            }
            SampleStatus::VerifyMismatch | SampleStatus::RestoreError => {
                counts.invalid += 1;
                counts.processed += 1;
            }
        }
    }
    counts
}

fn build_validation_confirmation(target: &TargetInfo) -> String {
    let mut lines = vec![
        format!(
            "DriveCk is about to validate {} ({}, {}).",
            device_display_name(target),
            format_bytes(target.size_bytes),
            target.path
        ),
        String::new(),
        "Important:".to_string(),
        "- Sampled regions will be written temporarily and restored afterwards.".to_string(),
        "- Close Explorer windows and any open files on the disk first.".to_string(),
    ];
    if target.is_mounted {
        lines.push(
            "- Mounted volumes on this disk will be dismounted before validation starts."
                .to_string(),
        );
    }
    lines.push("- A detailed text report will be available when the run finishes.".to_string());
    lines.push(String::new());
    lines.push("Continue?".to_string());
    lines.join("\r\n")
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

fn format_local_timestamp(timestamp: i64) -> String {
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|value| value.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;
    format!("{hours:02}:{minutes:02}:{secs:02}")
}

fn format_report_elapsed(report: &ValidationReport) -> String {
    let elapsed = report.finished_at.saturating_sub(report.started_at).max(0) as u64;
    format_duration(Duration::from_secs(elapsed))
}

fn current_elapsed_text(state: &AppState) -> String {
    if let Some(started) = state.validation_started_at {
        format_duration(started.elapsed())
    } else {
        state
            .last_elapsed_text
            .clone()
            .unwrap_or_else(|| "00:00:00".to_string())
    }
}

fn progress_basis_points(current: usize, total: usize) -> u32 {
    if total == 0 {
        0
    } else {
        ((current.min(total) * 1000) / total) as u32
    }
}

fn progress_percent_text(current: usize, total: usize) -> String {
    if total == 0 {
        "0.0%".to_string()
    } else {
        format!("{:.1}%", (current.min(total) as f64 * 100.0) / total as f64)
    }
}

fn title_case_phrase(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut new_word = true;
    for ch in text.chars() {
        if ch == ' ' || ch == '-' || ch == '_' {
            new_word = true;
            output.push(if ch == '_' { ' ' } else { ch });
            continue;
        }
        if new_word {
            output.extend(ch.to_uppercase());
            new_word = false;
        } else {
            output.push(ch);
        }
    }
    output
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

fn device_status_label(state: &AppState, target: &TargetInfo) -> String {
    if state.is_busy() {
        "Validating".to_string()
    } else if state
        .last_response
        .as_ref()
        .is_some_and(|response| response.target.path == target.path)
    {
        "Finished".to_string()
    } else if target.is_mounted {
        "Mounted".to_string()
    } else {
        "Ready".to_string()
    }
}

fn device_status_tone(state: &AppState, target: &TargetInfo) -> Tone {
    if state.is_busy() {
        Tone::Accent
    } else if state
        .last_response
        .as_ref()
        .is_some_and(|response| response.target.path == target.path)
    {
        validation_status_tone(state)
    } else if target.is_mounted {
        Tone::Warning
    } else {
        Tone::Success
    }
}

#[allow(dead_code)]
fn device_panel_note_text(state: &AppState, target: &TargetInfo) -> String {
    if state.is_busy() || state.last_response.is_some() {
        String::new()
    } else if target.is_mounted {
        format!("{} before validation.", state.status_text)
    } else {
        "Run validation to populate the summary and report.".to_string()
    }
}

unsafe fn footer_stage_text(state: &AppState) -> String {
    state.current_phase.clone()
}

unsafe fn footer_stage_tone(state: &AppState) -> Tone {
    if state.current_phase == "Waiting to start" {
        state.status_tone
    } else {
        validation_status_tone(state)
    }
}

fn validation_status_label(state: &AppState) -> String {
    if state.stop_requested {
        "Stopping".to_string()
    } else if state.is_busy() {
        "Validating".to_string()
    } else if let Some(response) = state.last_response.as_ref() {
        report_banner_title(&response.report).to_string()
    } else if unsafe { state.selected_target().is_some() } {
        "Ready".to_string()
    } else {
        "Idle".to_string()
    }
}

fn validation_status_tone(state: &AppState) -> Tone {
    if state.stop_requested {
        Tone::Warning
    } else if state.is_busy() {
        Tone::Accent
    } else if let Some(response) = state.last_response.as_ref() {
        report_banner_tone(&response.report)
    } else if unsafe { state.selected_target().is_some() } {
        Tone::Success
    } else {
        Tone::Neutral
    }
}

fn report_issue_count(report: &ValidationReport) -> usize {
    report.read_error_count
        + report.write_error_count
        + report.mismatch_count
        + report.restore_error_count
}

fn report_banner_title(report: &ValidationReport) -> &'static str {
    if report.restore_error_count != 0 {
        "Restore failure"
    } else if report.cancelled {
        "Cancelled"
    } else if report.mismatch_count != 0 {
        "Failed"
    } else if report.read_error_count != 0 || report.write_error_count != 0 {
        "I/O errors"
    } else if !report.completed_all_samples {
        "Incomplete"
    } else {
        "Passed"
    }
}

fn report_banner_subtitle(report: &ValidationReport) -> &'static str {
    if report.restore_error_count != 0 {
        "Some sampled regions could not be restored."
    } else if report.cancelled {
        "Stopped before every sample completed."
    } else if report.mismatch_count != 0 {
        "Mismatch indicators were detected."
    } else if report.read_error_count != 0 || report.write_error_count != 0 {
        "Read or write errors were detected."
    } else if !report.completed_all_samples {
        "Validation did not complete every sample."
    } else {
        "No issues detected."
    }
}

fn report_banner_tone(report: &ValidationReport) -> Tone {
    if report.restore_error_count != 0
        || report.mismatch_count != 0
        || report.read_error_count != 0
        || report.write_error_count != 0
    {
        Tone::Danger
    } else if report.cancelled || !report.completed_all_samples {
        Tone::Warning
    } else {
        Tone::Success
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
        "None".to_string()
    } else {
        parts.join(" · ")
    }
}

fn report_error_preview(target: &TargetInfo, error: &str) -> String {
    format!("DriveCk\r\nTarget: {}\r\n\r\n{}", target.path, error)
}

fn report_placeholder_text() -> &'static str {
    "The detailed report becomes available when validation finishes."
}

fn final_status_text(error: Option<&str>, report: &ValidationReport) -> (String, Tone) {
    if error.is_some() {
        if report.cancelled {
            ("Cancelled".to_string(), Tone::Warning)
        } else {
            ("Failed".to_string(), Tone::Danger)
        }
    } else if report.cancelled {
        ("Cancelled".to_string(), Tone::Warning)
    } else if report_issue_count(report) != 0 {
        ("Issues found".to_string(), Tone::Danger)
    } else if !report.completed_all_samples {
        ("Incomplete".to_string(), Tone::Warning)
    } else {
        ("Finished".to_string(), Tone::Success)
    }
}

extern "C" fn ffi_progress_callback(
    phase: *const c_char,
    current: usize,
    total: usize,
    _final_update: bool,
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
    let path = CString::new(path).map_err(|_| "Device path contains a null byte.".to_string())?;
    decode_envelope(
        driveck_ffi_inspect_target_json(path.as_ptr()),
        "driveck_ffi_inspect_target_json",
    )
}

fn ffi_unmount_target(path: &str) -> Result<TargetInfo, String> {
    let path = CString::new(path).map_err(|_| "Device path contains a null byte.".to_string())?;
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

fn decode_envelope<T: DeserializeOwned>(pointer: *mut c_char, label: &str) -> Result<T, String> {
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

unsafe fn register_window_class(
    hinstance: HINSTANCE,
    class_name: &str,
    proc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
) {
    let class_name_wide = wide(class_name);
    let wc = WNDCLASSW {
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
        hInstance: hinstance,
        lpszClassName: PCWSTR(class_name_wide.as_ptr()),
        lpfnWndProc: Some(proc),
        ..Default::default()
    };
    let _ = RegisterClassW(&wc);
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

fn normalize_windows_newlines(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

unsafe fn show_message(hwnd: HWND, title: &str, detail: &str, flags: MESSAGEBOX_STYLE) {
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
            WM_DRIVECK_PROGRESS => drop(Box::from_raw(message.lParam.0 as *mut ProgressPayload)),
            WM_DRIVECK_FINISHED => drop(Box::from_raw(message.lParam.0 as *mut FinishedPayload)),
            _ => {}
        }
    }
}

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    COLORREF(red as u32 | ((green as u32) << 8) | ((blue as u32) << 16))
}
