use super::layout::current_report_layout;
use super::*;

struct ReportWindowState {
    parent: HWND,
    edit: HWND,
    copy_button: HWND,
    save_button: HWND,
    close_button: HWND,
    ui_font: HGDIOBJ,
    mono_font: HGDIOBJ,
    report_ready: bool,
    report_text: String,
    response: Option<ValidationResponse>,
    target_path: Option<String>,
    language: Language,
}

pub(super) unsafe extern "system" fn report_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            LRESULT(1)
        }
        WM_CREATE => {
            if let Some(state) = report_state_mut(hwnd) {
                state.edit = create_control_ex(
                    "EDIT",
                    "",
                    hwnd,
                    0,
                    0,
                    0,
                    0,
                    IDC_REPORT_EDIT,
                    ws(ES_LEFT | ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY | ES_WANTRETURN)
                        | WS_BORDER
                        | WS_VSCROLL,
                    WS_EX_CLIENTEDGE,
                );
                state.copy_button = create_control(
                    "BUTTON",
                    report_copy_button_text(state.language),
                    hwnd,
                    0,
                    0,
                    0,
                    0,
                    IDC_REPORT_COPY,
                    ws(BS_PUSHBUTTON),
                );
                state.save_button = create_control(
                    "BUTTON",
                    report_save_button_text(state.language),
                    hwnd,
                    0,
                    0,
                    0,
                    0,
                    IDC_REPORT_SAVE,
                    ws(BS_PUSHBUTTON),
                );
                state.close_button = create_control(
                    "BUTTON",
                    close_button_text(state.language),
                    hwnd,
                    0,
                    0,
                    0,
                    0,
                    IDC_REPORT_CLOSE,
                    ws(BS_PUSHBUTTON),
                );

                let _ = SendMessageW(
                    state.edit,
                    WM_SETFONT,
                    Some(WPARAM(state.mono_font.0 as usize)),
                    Some(LPARAM(1)),
                );
                for button in [state.copy_button, state.save_button, state.close_button] {
                    let _ = SendMessageW(
                        button,
                        WM_SETFONT,
                        Some(WPARAM(state.ui_font.0 as usize)),
                        Some(LPARAM(1)),
                    );
                    let theme = wide("Explorer");
                    let _ = SetWindowTheme(button, PCWSTR(theme.as_ptr()), PCWSTR::null());
                }
                let theme = wide("Explorer");
                let _ = SetWindowTheme(state.edit, PCWSTR(theme.as_ptr()), PCWSTR::null());
                sync_report_window_controls(state);
                layout_report_window(hwnd);
            }
            LRESULT(0)
        }
        WM_SIZE => {
            layout_report_window(hwnd);
            redraw_window_now(hwnd);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            apply_suggested_dpi_rect(hwnd, lparam);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(lparam.0 as *mut MINMAXINFO);
            info.ptMinTrackSize.x = scale_for_window(hwnd, MIN_REPORT_WINDOW_WIDTH);
            info.ptMinTrackSize.y = scale_for_window(hwnd, MIN_REPORT_WINDOW_HEIGHT);
            LRESULT(0)
        }
        WM_COMMAND => {
            let control_id = (wparam.0 & 0xffff) as i32;
            if let Some(state) = report_state_mut(hwnd) {
                match control_id {
                    IDC_REPORT_COPY => copy_report_text(state),
                    IDC_REPORT_SAVE => {
                        if state.report_ready {
                            match save_report_text(hwnd, &state.report_text) {
                                Ok(()) => {}
                                Err(error) => {
                                    show_message(
                                        hwnd,
                                        failed_to_save_report_title(state.language),
                                        &error,
                                        MB_ICONERROR,
                                    );
                                }
                            }
                        }
                    }
                    IDC_REPORT_CLOSE => {
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_PAINT => {
            if let Some(state) = report_state_mut(hwnd) {
                paint_report_window(hwnd, state);
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Some(state_ptr) = take_report_state(hwnd) {
                let state = Box::from_raw(state_ptr);
                if let Some(parent) = state_mut(state.parent) {
                    if parent.report_window == Some(hwnd) {
                        parent.report_window = None;
                    }
                }
                drop(state);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

pub(super) unsafe fn open_report_window(state: &mut AppState) {
    if let Some(hwnd) = state.report_window {
        sync_report_window_from_main_state(state);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        return;
    }

    let window_state = Box::new(ReportWindowState {
        parent: state.hwnd,
        edit: HWND::default(),
        copy_button: HWND::default(),
        save_button: HWND::default(),
        close_button: HWND::default(),
        ui_font: state.ui_font,
        mono_font: state.mono_font,
        report_ready: state.report_text.is_some(),
        report_text: state
            .report_text
            .clone()
            .unwrap_or_else(|| report_placeholder_text(state.language).to_string()),
        response: state.last_response.clone(),
        target_path: state.last_report_target_path.clone(),
        language: state.language,
    });

    let class_name = wide(REPORT_CLASS_NAME);
    let title = wide(report_window_title(state.language));
    let hwnd = CreateWindowExW(
        Default::default(),
        PCWSTR(class_name.as_ptr()),
        PCWSTR(title.as_ptr()),
        WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        860,
        660,
        Some(state.hwnd),
        None,
        Some(HINSTANCE(GetModuleHandleW(None).unwrap().0)),
        Some(Box::into_raw(window_state) as *const c_void),
    )
    .expect("create report window");
    set_text(hwnd, report_window_title(state.language));
    center_window(hwnd, Some(state.hwnd), 860, 660);
    state.report_window = Some(hwnd);
    let _ = ShowWindow(hwnd, SW_SHOW);
}

pub(super) unsafe fn sync_report_window_from_main_state(state: &AppState) {
    let Some(report_hwnd) = state.report_window else {
        return;
    };
    let Some(report_state) = report_state_mut(report_hwnd) else {
        return;
    };

    report_state.report_ready = state.report_text.is_some();
    report_state.language = state.language;
    report_state.report_text = state
        .report_text
        .clone()
        .unwrap_or_else(|| report_placeholder_text(state.language).to_string());
    report_state.response = state.last_response.clone();
    report_state.target_path = state.last_report_target_path.clone();
    set_text(report_hwnd, report_window_title(state.language));
    sync_report_window_controls(report_state);
    let _ = InvalidateRect(Some(report_hwnd), None, true);
}

unsafe fn sync_report_window_controls(state: &ReportWindowState) {
    let normalized = normalize_windows_newlines(&state.report_text);
    set_text(state.edit, &normalized);
    set_text(state.copy_button, report_copy_button_text(state.language));
    set_text(state.save_button, report_save_button_text(state.language));
    set_text(state.close_button, close_button_text(state.language));
    enable(state.save_button, state.report_ready);
}

unsafe fn copy_report_text(state: &ReportWindowState) {
    send_message(state.edit, EM_SETSEL, 0, -1);
    send_message(state.edit, WM_COPY, 0, 0);
}

unsafe fn layout_report_window(hwnd: HWND) {
    let Some(state) = report_state_mut(hwnd) else {
        return;
    };
    let layout = current_report_layout(hwnd);
    for (control, rect) in [
        (state.edit, layout.edit),
        (state.copy_button, layout.copy_button),
        (state.save_button, layout.save_button),
        (state.close_button, layout.close_button),
    ] {
        let _ = MoveWindow(
            control,
            rect.left,
            rect.top,
            rect_width(rect),
            rect_height(rect),
            true,
        );
    }
}

unsafe fn paint_report_window(hwnd: HWND, state: &ReportWindowState) {
    let mut paint = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut paint);
    let client = current_client_rect(hwnd);
    let layout = current_report_layout(hwnd);

    let back_dc = CreateCompatibleDC(Some(hdc));
    let bitmap = CreateCompatibleBitmap(hdc, rect_width(client), rect_height(client));
    let old_bitmap = SelectObject(back_dc, HGDIOBJ(bitmap.0));

    fill_rect_color(back_dc, &client, APP_BG);
    draw_panel(back_dc, &layout.header_panel);

    let header_content = inset_rect(layout.header_panel, 14, 14);
    let title = state
        .response
        .as_ref()
        .map(|response| report_banner_title(state.language, &response.report))
        .unwrap_or(report_preview_title(state.language));
    let subtitle = state
        .response
        .as_ref()
        .map(|response| report_banner_subtitle(state.language, &response.report).to_string())
        .unwrap_or_else(|| shared_formatter_output_text(state.language).to_string());
    let tone = state
        .response
        .as_ref()
        .map(|response| report_banner_tone(&response.report))
        .unwrap_or(if state.report_ready {
            Tone::Accent
        } else {
            Tone::Neutral
        });

    let (banner_bg, banner_fg) = tone_palette(tone);
    let banner_rect = make_rect(
        header_content.left,
        header_content.top,
        rect_width(header_content),
        52,
    );
    fill_rect_color(back_dc, &banner_rect, banner_bg);
    frame_rect_color(back_dc, &banner_rect, banner_bg);

    let banner_inner = inset_rect(banner_rect, 12, 8);
    let title_rect = make_rect(
        banner_inner.left,
        banner_inner.top,
        rect_width(banner_inner),
        18,
    );
    draw_text_block(
        back_dc,
        title_rect,
        title,
        banner_fg,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        state.ui_font,
    );
    let subtitle_rect = make_rect(
        banner_inner.left,
        title_rect.bottom + 2,
        rect_width(banner_inner),
        (banner_inner.bottom - title_rect.bottom - 2).max(0),
    );
    draw_text_block(
        back_dc,
        subtitle_rect,
        &subtitle,
        banner_fg,
        DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
        state.ui_font,
    );

    let detail_top = banner_rect.bottom + 6;
    let target_value = state
        .response
        .as_ref()
        .map(|response| device_display_name(&response.target))
        .or_else(|| state.target_path.clone())
        .unwrap_or_else(|| "-".to_string());
    let detail_width = rect_width(header_content);
    let detail_gap = 12;
    let left_width = ((detail_width - detail_gap) / 2).max(0);
    let right_width = (detail_width - left_width - detail_gap).max(0);
    draw_metric_row(
        back_dc,
        make_rect(header_content.left, detail_top, left_width, 22),
        device_label_text(state.language),
        &target_value,
        state.ui_font,
    );
    let completed_value = state
        .response
        .as_ref()
        .map(|response| format_local_timestamp(response.report.finished_at))
        .unwrap_or_else(|| "-".to_string());
    draw_metric_row(
        back_dc,
        make_rect(
            header_content.left + left_width + detail_gap,
            detail_top,
            right_width,
            22,
        ),
        completed_label_text(state.language),
        &completed_value,
        state.ui_font,
    );

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

unsafe fn report_state_mut(hwnd: HWND) -> Option<&'static mut ReportWindowState> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ReportWindowState;
    (!ptr.is_null()).then_some(&mut *ptr)
}

unsafe fn take_report_state(hwnd: HWND) -> Option<*mut ReportWindowState> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ReportWindowState;
    if ptr.is_null() {
        None
    } else {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        Some(ptr)
    }
}
