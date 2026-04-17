use std::ffi::{CStr, CString, c_char, c_void};

use driveck_core::{
    ProgressObserver, ProgressUpdate, ValidationOptions, ValidationResponse, collect_targets,
    discover_target, format_report_text, validate_target_with_callbacks,
};
use serde::Serialize;

type ProgressCallback = Option<
    extern "C" fn(
        phase: *const c_char,
        current: usize,
        total: usize,
        final_update: bool,
        user_data: *mut c_void,
    ),
>;
type CancelCallback = Option<extern "C" fn(user_data: *mut c_void) -> bool>;

#[derive(Serialize)]
struct Envelope<T> {
    ok: bool,
    data: Option<T>,
    error: Option<String>,
}

struct FfiProgress {
    callback: ProgressCallback,
    user_data: *mut c_void,
}

impl ProgressObserver for FfiProgress {
    fn on_progress(&mut self, update: ProgressUpdate) {
        if let Some(callback) = self.callback {
            if let Ok(phase) = CString::new(update.phase) {
                callback(
                    phase.as_ptr(),
                    update.current,
                    update.total,
                    update.final_update,
                    self.user_data,
                );
            }
        }
    }
}

fn response_json<T: Serialize>(value: Result<T, String>) -> *mut c_char {
    let payload = match value {
        Ok(data) => Envelope {
            ok: true,
            data: Some(data),
            error: None,
        },
        Err(error) => Envelope::<T> {
            ok: false,
            data: None,
            error: Some(error),
        },
    };

    let json = serde_json::to_string(&payload).unwrap_or_else(|error| {
        serde_json::json!({
            "ok": false,
            "data": serde_json::Value::Null,
            "error": error.to_string(),
        })
        .to_string()
    });
    CString::new(json).unwrap().into_raw()
}

fn decode_input(input: *const c_char, label: &str) -> Result<String, String> {
    if input.is_null() {
        return Err(format!("{label} pointer is null."));
    }
    let text = unsafe { CStr::from_ptr(input) }
        .to_str()
        .map_err(|_| format!("{label} must be valid UTF-8."))?;
    Ok(text.to_string())
}

#[unsafe(no_mangle)]
pub extern "C" fn driveck_ffi_free_string(value: *mut c_char) {
    if value.is_null() {
        return;
    }
    unsafe {
        let _ = CString::from_raw(value);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn driveck_ffi_list_targets_json() -> *mut c_char {
    response_json(collect_targets().map_err(|error| error.message))
}

#[unsafe(no_mangle)]
pub extern "C" fn driveck_ffi_discover_target_json(path: *const c_char) -> *mut c_char {
    response_json(
        decode_input(path, "path")
            .and_then(|path| discover_target(path).map_err(|error| error.message)),
    )
}

#[unsafe(no_mangle)]
pub extern "C" fn driveck_ffi_validate_path_json(
    path: *const c_char,
    seed_set: bool,
    seed: u64,
    progress_callback: ProgressCallback,
    cancel_callback: CancelCallback,
    user_data: *mut c_void,
) -> *mut c_char {
    response_json::<ValidationResponse>((|| {
        let path = decode_input(path, "path")?;
        let target = discover_target(&path).map_err(|error| error.message)?;
        let mut progress_bridge = FfiProgress {
            callback: progress_callback,
            user_data,
        };
        let cancel_bridge = || cancel_callback.is_some_and(|callback| callback(user_data));
        let report = validate_target_with_callbacks(
            &target,
            &ValidationOptions {
                seed: seed_set.then_some(seed),
            },
            Some(&mut progress_bridge),
            Some(&cancel_bridge),
        )
        .map_err(|error| error.message)?;
        Ok(ValidationResponse { target, report })
    })())
}

#[unsafe(no_mangle)]
pub extern "C" fn driveck_ffi_format_report_text_json(
    validation_json: *const c_char,
) -> *mut c_char {
    response_json::<String>((|| {
        let validation_json = decode_input(validation_json, "validation_json")?;
        let payload: ValidationResponse =
            serde_json::from_str(&validation_json).map_err(|error| error.to_string())?;
        Ok(format_report_text(&payload.target, &payload.report))
    })())
}
