use std::ffi::{CStr, CString, c_char, c_void};

use driveck_core::{
    ProgressObserver, ProgressUpdate, TargetInfo, ValidationFailure, ValidationOptions,
    ValidationResponse, collect_targets, discover_target, format_report_text,
    validate_target_with_callbacks,
};
use serde::{Deserialize, Serialize};

type ProgressCallback = Option<
    extern "C" fn(
        phase: *const c_char,
        current: usize,
        total: usize,
        final_update: bool,
        sample_index: isize,
        sample_status: i32,
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

#[derive(Deserialize)]
struct ValidationRequest {
    target: TargetInfo,
    #[serde(default)]
    options: ValidationOptions,
}

#[derive(Serialize)]
struct ValidationExecutionResult {
    response: Option<ValidationResponse>,
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
                    update.sample_index.map(|index| index as isize).unwrap_or(-1),
                    sample_status_code(update.sample_status),
                    self.user_data,
                );
            }
        }
    }
}

fn sample_status_code(status: Option<driveck_core::SampleStatus>) -> i32 {
    match status {
        Some(driveck_core::SampleStatus::Untested) => 0,
        Some(driveck_core::SampleStatus::Ok) => 1,
        Some(driveck_core::SampleStatus::ReadError) => 2,
        Some(driveck_core::SampleStatus::WriteError) => 3,
        Some(driveck_core::SampleStatus::VerifyMismatch) => 4,
        Some(driveck_core::SampleStatus::RestoreError) => 5,
        None => -1,
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

fn decode_json<T: for<'de> Deserialize<'de>>(
    input: *const c_char,
    label: &str,
) -> Result<T, String> {
    let text = decode_input(input, label)?;
    serde_json::from_str(&text).map_err(|error| format!("{label} is invalid JSON: {error}"))
}

fn execute_validation(
    target: TargetInfo,
    options: ValidationOptions,
    progress_callback: ProgressCallback,
    cancel_callback: CancelCallback,
    user_data: *mut c_void,
) -> Result<ValidationExecutionResult, String> {
    let mut progress_bridge = FfiProgress {
        callback: progress_callback,
        user_data,
    };
    let cancel_bridge = || cancel_callback.is_some_and(|callback| callback(user_data));
    let result = validate_target_with_callbacks(
        &target,
        &options,
        Some(&mut progress_bridge),
        Some(&cancel_bridge),
    );
    Ok(match result {
        Ok(report) => ValidationExecutionResult {
            response: Some(ValidationResponse { target, report }),
            error: None,
        },
        Err(ValidationFailure { message, report }) => ValidationExecutionResult {
            response: report.map(|report| ValidationResponse { target, report }),
            error: Some(message),
        },
    })
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
    response_json::<ValidationExecutionResult>((|| {
        let path = decode_input(path, "path")?;
        let target = discover_target(&path).map_err(|error| error.message)?;
        execute_validation(
            target,
            ValidationOptions {
                seed: seed_set.then_some(seed),
            },
            progress_callback,
            cancel_callback,
            user_data,
        )
    })())
}

#[unsafe(no_mangle)]
pub extern "C" fn driveck_ffi_validate_target_json(
    target_json: *const c_char,
    seed_set: bool,
    seed: u64,
    progress_callback: ProgressCallback,
    cancel_callback: CancelCallback,
    user_data: *mut c_void,
) -> *mut c_char {
    response_json::<ValidationExecutionResult>((|| {
        let mut request: ValidationRequest = decode_json(target_json, "target_json")?;
        if seed_set {
            request.options.seed = Some(seed);
        }
        execute_validation(
            request.target,
            request.options,
            progress_callback,
            cancel_callback,
            user_data,
        )
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
