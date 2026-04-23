use super::layout::current_about_layout;
use super::*;
use windows::Win32::UI::Shell::ShellExecuteW;

struct AboutWindowState {
    parent: HWND,
    github_button: HWND,
    close_button: HWND,
    ui_font: HGDIOBJ,
}

pub(super) unsafe extern "system" fn about_window_proc(
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
            if let Some(state) = about_state_mut(hwnd) {
                state.github_button = create_control(
                    "BUTTON",
                    LABEL_GITHUB,
                    hwnd,
                    0,
                    0,
                    0,
                    0,
                    IDC_ABOUT_OPEN_GITHUB,
                    ws(BS_PUSHBUTTON),
                );
                state.close_button = create_control(
                    "BUTTON",
                    LABEL_CLOSE,
                    hwnd,
                    0,
                    0,
                    0,
                    0,
                    IDC_ABOUT_CLOSE,
                    ws(BS_PUSHBUTTON),
                );
                for button in [state.github_button, state.close_button] {
                    let _ = SendMessageW(
                        button,
                        WM_SETFONT,
                        Some(WPARAM(state.ui_font.0 as usize)),
                        Some(LPARAM(1)),
                    );
                    let theme = wide("Explorer");
                    let _ = SetWindowTheme(button, PCWSTR(theme.as_ptr()), PCWSTR::null());
                }
                layout_about_window(hwnd);
            }
            LRESULT(0)
        }
        WM_SIZE => {
            layout_about_window(hwnd);
            redraw_window_now(hwnd);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            apply_suggested_dpi_rect(hwnd, lparam);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(lparam.0 as *mut MINMAXINFO);
            info.ptMinTrackSize.x = scale_for_window(hwnd, MIN_ABOUT_WINDOW_WIDTH);
            info.ptMinTrackSize.y = scale_for_window(hwnd, MIN_ABOUT_WINDOW_HEIGHT);
            LRESULT(0)
        }
        WM_COMMAND => {
            let control_id = (wparam.0 & 0xffff) as i32;
            match control_id {
                IDC_ABOUT_OPEN_GITHUB => {
                    open_repository_url(hwnd);
                }
                IDC_ABOUT_CLOSE => {
                    let _ = DestroyWindow(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_PAINT => {
            if let Some(state) = about_state_mut(hwnd) {
                paint_about_window(hwnd, state);
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
            if let Some(state_ptr) = take_about_state(hwnd) {
                let state = Box::from_raw(state_ptr);
                if let Some(parent) = state_mut(state.parent) {
                    if parent.about_window == Some(hwnd) {
                        parent.about_window = None;
                    }
                }
                drop(state);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

pub(super) unsafe fn open_about_window(state: &mut AppState) {
    if let Some(hwnd) = state.about_window {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        return;
    }

    let window_state = Box::new(AboutWindowState {
        parent: state.hwnd,
        github_button: HWND::default(),
        close_button: HWND::default(),
        ui_font: state.ui_font,
    });

    let class_name = wide(ABOUT_CLASS_NAME);
    let title = wide("About Driveck");
    let hwnd = CreateWindowExW(
        Default::default(),
        PCWSTR(class_name.as_ptr()),
        PCWSTR(title.as_ptr()),
        WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        560,
        380,
        Some(state.hwnd),
        None,
        Some(HINSTANCE(GetModuleHandleW(None).unwrap().0)),
        Some(Box::into_raw(window_state) as *const c_void),
    )
    .expect("create about window");
    set_text(hwnd, "About Driveck");
    center_window(hwnd, Some(state.hwnd), 560, 380);
    state.about_window = Some(hwnd);
    let _ = ShowWindow(hwnd, SW_SHOW);
}

unsafe fn layout_about_window(hwnd: HWND) {
    let Some(state) = about_state_mut(hwnd) else {
        return;
    };
    let layout = current_about_layout(hwnd);
    for (control, rect) in [
        (state.github_button, layout.github_button),
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

unsafe fn open_repository_url(hwnd: HWND) {
    let operation = wide("open");
    let url = wide(APP_REPOSITORY_URL);
    let _ = ShellExecuteW(
        Some(hwnd),
        PCWSTR(operation.as_ptr()),
        PCWSTR(url.as_ptr()),
        PCWSTR::null(),
        PCWSTR::null(),
        SW_SHOWNORMAL,
    );
}

unsafe fn paint_about_window(hwnd: HWND, state: &AboutWindowState) {
    let mut paint = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut paint);
    let client = current_client_rect(hwnd);
    let layout = current_about_layout(hwnd);

    let back_dc = CreateCompatibleDC(Some(hdc));
    let bitmap = CreateCompatibleBitmap(hdc, rect_width(client), rect_height(client));
    let old_bitmap = SelectObject(back_dc, HGDIOBJ(bitmap.0));

    fill_rect_color(back_dc, &client, APP_BG);
    draw_panel(back_dc, &layout.hero_panel);

    let content = inset_rect(layout.hero_panel, 20, 20);
    let title_rect = make_rect(content.left, content.top, rect_width(content), 24);
    draw_text_block(
        back_dc,
        title_rect,
        LABEL_ABOUT_TITLE,
        TEXT_PRIMARY,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        state.ui_font,
    );

    let desc_rect = make_rect(
        content.left,
        title_rect.bottom + 12,
        rect_width(content),
        40,
    );
    draw_text_block(
        back_dc,
        desc_rect,
        "Windows utility for validating removable storage capacity and integrity.",
        TEXT_MUTED,
        DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
        state.ui_font,
    );

    let mut row_top = desc_rect.bottom + 14;
    for (label, value) in [
        ("Version", env!("CARGO_PKG_VERSION").to_string()),
        ("Frontend", "Rust + Win32".to_string()),
        ("License", env!("CARGO_PKG_LICENSE").to_string()),
        ("Repository", APP_REPOSITORY_URL.to_string()),
    ] {
        draw_metric_row(
            back_dc,
            make_rect(content.left, row_top, rect_width(content), 22),
            label,
            &value,
            state.ui_font,
        );
        row_top += 28;
    }

    let note_rect = make_rect(content.left, row_top + 8, rect_width(content), 40);
    draw_text_block(
        back_dc,
        note_rect,
        "Shared Rust engine with a native Win32 dashboard, live sample map, and report viewer.",
        TEXT_MUTED,
        DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
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

unsafe fn about_state_mut(hwnd: HWND) -> Option<&'static mut AboutWindowState> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AboutWindowState;
    (!ptr.is_null()).then_some(&mut *ptr)
}

unsafe fn take_about_state(hwnd: HWND) -> Option<*mut AboutWindowState> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AboutWindowState;
    if ptr.is_null() {
        None
    } else {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        Some(ptr)
    }
}
