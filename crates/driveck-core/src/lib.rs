mod error;
mod formatting;
mod model;
mod platform;
mod report;
mod template;
mod validation;

pub use error::DriveCkError;
pub use formatting::{
    format_basis_points, format_bytes, format_decimal_millis, format_local_timestamp,
    report_has_failures, report_verdict, right_align_cell, sample_status_glyph, sample_status_name,
};
pub use model::{
    CancelObserver, DRIVECK_MAP_COLUMNS, DRIVECK_MAP_ROWS, DRIVECK_MAX_REGION_SIZE,
    DRIVECK_MIN_REGION_SIZE, DRIVECK_SAMPLE_COUNT, DRIVECK_TIMING_CAPACITY, ProgressObserver,
    ProgressUpdate, SampleStatus, TargetInfo, TargetKind, TimingSeries, TimingSummary,
    ValidationOptions, ValidationReport, ValidationResponse,
};
pub use report::{format_report_text, save_report, summarize_timings};
pub use template::expand_template;
pub use validation::{
    ValidationFailure, build_sample_order, validate_target, validate_target_with_callbacks,
};

pub fn collect_targets() -> Result<Vec<TargetInfo>, DriveCkError> {
    platform::collect_targets()
}

pub fn discover_target(path: impl AsRef<std::path::Path>) -> Result<TargetInfo, DriveCkError> {
    platform::discover_target(path.as_ref())
}

pub fn inspect_target(path: impl AsRef<std::path::Path>) -> Result<TargetInfo, DriveCkError> {
    platform::inspect_target(path.as_ref())
}

pub fn unmount_target(path: impl AsRef<std::path::Path>) -> Result<TargetInfo, DriveCkError> {
    platform::unmount_target(path.as_ref())
}

pub fn release_unmount_target(path: impl AsRef<std::path::Path>) -> Result<(), DriveCkError> {
    platform::release_unmount_target(path.as_ref())
}
