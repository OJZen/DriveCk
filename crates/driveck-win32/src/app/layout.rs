use std::{ffi::c_void, mem::size_of};

use windows::Win32::{
    Foundation::{HWND, LPARAM, RECT},
    Graphics::Gdi::{
        CreateFontIndirectW, DEFAULT_GUI_FONT, GetStockObject, HGDIOBJ, RDW_ALLCHILDREN, RDW_ERASE,
        RDW_INVALIDATE, RDW_UPDATENOW, RedrawWindow, UpdateWindow,
    },
    UI::{
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::{
            GetClientRect, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SWP_NOACTIVATE,
            SWP_NOZORDER, SetWindowPos, SystemParametersInfoW,
        },
    },
};

#[derive(Clone, Copy)]
pub(crate) struct Layout {
    pub(crate) header_panel: RECT,
    #[allow(dead_code)]
    pub(crate) header_label: RECT,
    pub(crate) combo: RECT,
    pub(crate) refresh_button: RECT,
    pub(crate) validate_button: RECT,
    pub(crate) stop_button: RECT,
    pub(crate) save_button: RECT,
    pub(crate) about_button: RECT,
    #[allow(dead_code)]
    pub(crate) device_panel: RECT,
    pub(crate) map_panel: RECT,
    #[allow(dead_code)]
    pub(crate) summary_panel: RECT,
    pub(crate) report_panel: RECT,
    pub(crate) report_scrollbar: RECT,
    pub(crate) report_button: RECT,
    pub(crate) footer_panel: RECT,
    pub(crate) status_label: RECT,
    pub(crate) progress_label: RECT,
    pub(crate) progress: RECT,
    pub(crate) percent: RECT,
    pub(crate) elapsed: RECT,
}

#[derive(Clone, Copy)]
pub(crate) struct ReportLayout {
    pub(crate) header_panel: RECT,
    pub(crate) edit: RECT,
    pub(crate) copy_button: RECT,
    pub(crate) save_button: RECT,
    pub(crate) close_button: RECT,
}

#[derive(Clone, Copy)]
pub(crate) struct AboutLayout {
    pub(crate) hero_panel: RECT,
    pub(crate) github_button: RECT,
    pub(crate) close_button: RECT,
}

pub(crate) fn current_layout(hwnd: HWND) -> Layout {
    let client = current_client_rect(hwnd);
    let width = rect_width(client);
    let height = rect_height(client);

    let margin = scale_for_window(hwnd, 8);
    let gap = scale_for_window(hwnd, 8);
    let header_height = scale_for_window(hwnd, 104);
    let footer_height = scale_for_window(hwnd, 44);
    let control_height = scale_for_window(hwnd, 31);

    let header_panel = make_rect(margin, margin, width - margin * 2, header_height);
    let header_label = make_rect(0, 0, 0, 0);

    let controls_top = header_panel.top + scale_for_window(hwnd, 12);
    let controls_left = header_panel.left + scale_for_window(hwnd, 12);
    let controls_right = header_panel.right - scale_for_window(hwnd, 12);
    let button_gap = scale_for_window(hwnd, 8);
    let refresh_width = scale_for_window(hwnd, 86);
    let validate_width = scale_for_window(hwnd, 96);
    let open_report_width = scale_for_window(hwnd, 96);
    let save_width = scale_for_window(hwnd, 98);
    let about_width = scale_for_window(hwnd, 72);
    let combo_width = (controls_right
        - controls_left
        - button_gap * 3
        - refresh_width
        - validate_width
        - about_width)
        .max(scale_for_window(hwnd, 240));

    let combo = make_rect(controls_left, controls_top, combo_width, control_height);
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
    let stop_button = validate_button;
    let about_button = make_rect(
        validate_button.right + button_gap,
        controls_top,
        about_width,
        control_height,
    );

    let footer_panel = make_rect(
        margin,
        height - margin - footer_height,
        width - margin * 2,
        footer_height,
    );

    let content_top = header_panel.bottom + gap;
    let content_bottom = footer_panel.top - gap;
    let content_height = content_bottom - content_top;
    let content_width = width - margin * 2;
    let right_width = (((content_width - gap) * 27 / 100) + scale_for_window(hwnd, 50))
        .clamp(scale_for_window(hwnd, 310), scale_for_window(hwnd, 390));
    let center_width = (content_width - right_width - gap).max(scale_for_window(hwnd, 520));

    let center_x = margin;
    let right_x = center_x + center_width + gap;

    let device_panel = make_rect(0, 0, 0, 0);
    let summary_panel = make_rect(0, 0, 0, 0);
    let map_panel = make_rect(center_x, content_top, center_width, content_height);
    let report_panel = make_rect(right_x, content_top, right_width, content_height);

    let summary_action_bottom = report_panel.bottom - scale_for_window(hwnd, 14);
    let summary_button_height = scale_for_window(hwnd, 30);
    let summary_button_top = summary_action_bottom - summary_button_height;
    let report_scrollbar_width = scale_for_window(hwnd, 14);
    let report_scrollbar_top = report_panel.top + scale_for_window(hwnd, 52);
    let report_scrollbar_bottom = summary_button_top - scale_for_window(hwnd, 12);
    let report_scrollbar = make_rect(
        report_panel.right - scale_for_window(hwnd, 14) - report_scrollbar_width,
        report_scrollbar_top,
        report_scrollbar_width,
        (report_scrollbar_bottom - report_scrollbar_top).max(scale_for_window(hwnd, 80)),
    );
    let save_button = make_rect(
        report_panel.right - scale_for_window(hwnd, 14) - save_width,
        summary_button_top,
        save_width,
        summary_button_height,
    );
    let report_button = make_rect(
        save_button.left - button_gap - open_report_width,
        summary_button_top,
        open_report_width,
        summary_button_height,
    );
    let footer_inner_left = footer_panel.left + scale_for_window(hwnd, 10);
    let footer_inner_right = footer_panel.right - scale_for_window(hwnd, 10);

    let status_label = make_rect(
        footer_inner_left,
        footer_panel.top + scale_for_window(hwnd, 13),
        scale_for_window(hwnd, 220),
        scale_for_window(hwnd, 16),
    );
    let elapsed = make_rect(
        footer_inner_right - scale_for_window(hwnd, 112),
        footer_panel.top + scale_for_window(hwnd, 13),
        scale_for_window(hwnd, 112),
        scale_for_window(hwnd, 16),
    );
    let percent = make_rect(
        elapsed.left - scale_for_window(hwnd, 52),
        footer_panel.top + scale_for_window(hwnd, 13),
        scale_for_window(hwnd, 40),
        scale_for_window(hwnd, 16),
    );
    let progress_width = ((rect_width(footer_panel) * 30) / 100)
        .clamp(scale_for_window(hwnd, 220), scale_for_window(hwnd, 360));
    let progress_left = footer_panel.left + (rect_width(footer_panel) - progress_width) / 2;
    let progress = make_rect(
        progress_left,
        footer_panel.top + scale_for_window(hwnd, 12),
        progress_width,
        scale_for_window(hwnd, 16),
    );
    let progress_label = make_rect(
        progress.left - scale_for_window(hwnd, 58),
        footer_panel.top + scale_for_window(hwnd, 13),
        scale_for_window(hwnd, 52),
        scale_for_window(hwnd, 16),
    );

    Layout {
        header_panel,
        header_label,
        combo,
        refresh_button,
        validate_button,
        stop_button,
        save_button,
        about_button,
        device_panel,
        map_panel,
        summary_panel,
        report_panel,
        report_scrollbar,
        report_button,
        footer_panel,
        status_label,
        progress_label,
        progress,
        percent,
        elapsed,
    }
}

pub(crate) fn current_report_layout(hwnd: HWND) -> ReportLayout {
    let client = current_client_rect(hwnd);
    let width = rect_width(client);
    let height = rect_height(client);
    let margin = scale_for_window(hwnd, 14);
    let gap = scale_for_window(hwnd, 10);
    let button_width = scale_for_window(hwnd, 96);
    let button_height = scale_for_window(hwnd, 32);
    let footer_y = height - margin - button_height;
    let header_panel = make_rect(
        margin,
        margin,
        width - margin * 2,
        scale_for_window(hwnd, 112),
    );
    let edit_top = header_panel.bottom + gap;
    let edit = make_rect(
        margin,
        edit_top,
        width - margin * 2,
        footer_y - edit_top - gap,
    );
    let close_button = make_rect(
        width - margin - button_width,
        footer_y,
        button_width,
        button_height,
    );
    let save_button = make_rect(
        close_button.left - gap - button_width,
        footer_y,
        button_width,
        button_height,
    );
    let copy_button = make_rect(
        save_button.left - gap - button_width,
        footer_y,
        button_width,
        button_height,
    );

    ReportLayout {
        header_panel,
        edit,
        copy_button,
        save_button,
        close_button,
    }
}

pub(crate) fn current_about_layout(hwnd: HWND) -> AboutLayout {
    let client = current_client_rect(hwnd);
    let width = rect_width(client);
    let height = rect_height(client);
    let margin = scale_for_window(hwnd, 12);
    let gap = scale_for_window(hwnd, 8);
    let button_width = scale_for_window(hwnd, 104);
    let button_height = scale_for_window(hwnd, 32);
    let close_button = make_rect(
        width - margin - button_width,
        height - margin - button_height,
        button_width,
        button_height,
    );
    let github_button = make_rect(
        close_button.left - gap - scale_for_window(hwnd, 128),
        close_button.top,
        scale_for_window(hwnd, 128),
        button_height,
    );
    let hero_panel = make_rect(
        margin,
        margin,
        width - margin * 2,
        height - margin * 2 - button_height - gap,
    );

    AboutLayout {
        hero_panel,
        github_button,
        close_button,
    }
}

pub(crate) fn current_client_rect(hwnd: HWND) -> RECT {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    rect
}

pub(crate) fn inset_rect(rect: RECT, dx: i32, dy: i32) -> RECT {
    RECT {
        left: rect.left + dx,
        top: rect.top + dy,
        right: rect.right - dx,
        bottom: rect.bottom - dy,
    }
}

pub(crate) fn split_four(rect: RECT, gap: i32) -> [RECT; 4] {
    let width = (rect_width(rect) - gap * 3) / 4;
    [
        make_rect(rect.left, rect.top, width, rect_height(rect)),
        make_rect(rect.left + width + gap, rect.top, width, rect_height(rect)),
        make_rect(
            rect.left + (width + gap) * 2,
            rect.top,
            width,
            rect_height(rect),
        ),
        make_rect(
            rect.left + (width + gap) * 3,
            rect.top,
            rect.right - (rect.left + (width + gap) * 3),
            rect_height(rect),
        ),
    ]
}

pub(crate) unsafe fn load_system_ui_font() -> (HGDIOBJ, bool) {
    let mut metrics = NONCLIENTMETRICSW::default();
    metrics.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
    if SystemParametersInfoW(
        SPI_GETNONCLIENTMETRICS,
        metrics.cbSize,
        Some(&mut metrics as *mut _ as *mut c_void),
        Default::default(),
    )
    .is_ok()
    {
        let font = CreateFontIndirectW(&metrics.lfMessageFont);
        if !font.0.is_null() {
            return (HGDIOBJ(font.0), true);
        }
    }
    (GetStockObject(DEFAULT_GUI_FONT), false)
}

pub(crate) fn make_rect(x: i32, y: i32, width: i32, height: i32) -> RECT {
    RECT {
        left: x,
        top: y,
        right: x + width.max(0),
        bottom: y + height.max(0),
    }
}

pub(crate) fn rect_width(rect: RECT) -> i32 {
    rect.right - rect.left
}

pub(crate) fn rect_height(rect: RECT) -> i32 {
    rect.bottom - rect.top
}

pub(crate) fn point_in_rect(rect: RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

pub(crate) fn dpi_for_window(hwnd: HWND) -> i32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) } as i32;
    if dpi <= 0 { 96 } else { dpi }
}

pub(crate) fn scale_for_window(hwnd: HWND, value: i32) -> i32 {
    let dpi = dpi_for_window(hwnd);
    ((value * dpi) + 48) / 96
}

pub(crate) unsafe fn redraw_window_now(hwnd: HWND) {
    let _ = RedrawWindow(
        Some(hwnd),
        None,
        None,
        RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW,
    );
    let _ = UpdateWindow(hwnd);
}

pub(crate) unsafe fn apply_suggested_dpi_rect(hwnd: HWND, lparam: LPARAM) {
    let suggested = *(lparam.0 as *const RECT);
    let _ = SetWindowPos(
        hwnd,
        None,
        suggested.left,
        suggested.top,
        rect_width(suggested),
        rect_height(suggested),
        SWP_NOZORDER | SWP_NOACTIVATE,
    );
}
