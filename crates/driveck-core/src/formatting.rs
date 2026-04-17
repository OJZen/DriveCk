use chrono::{Local, TimeZone};

use crate::model::{SampleStatus, ValidationReport};

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];

    let mut value = bytes as f64;
    let mut unit_index = 0usize;
    while value >= 1024.0 && unit_index + 1 < UNITS.len() {
        value /= 1024.0;
        unit_index += 1;
    }
    format!("{value:.2} {}", UNITS[unit_index])
}

pub fn format_local_timestamp(timestamp: i64) -> String {
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|value| value.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "(unavailable)".to_string())
}

pub fn sample_status_name(status: SampleStatus) -> &'static str {
    match status {
        SampleStatus::Ok => "ok",
        SampleStatus::ReadError => "read error",
        SampleStatus::WriteError => "write error",
        SampleStatus::VerifyMismatch => "verify mismatch",
        SampleStatus::RestoreError => "restore error",
        SampleStatus::Untested => "untested",
    }
}

pub fn sample_status_glyph(status: SampleStatus) -> char {
    match status {
        SampleStatus::Ok => '.',
        SampleStatus::ReadError => 'R',
        SampleStatus::WriteError => 'W',
        SampleStatus::VerifyMismatch => 'M',
        SampleStatus::RestoreError => '!',
        SampleStatus::Untested => '?',
    }
}

pub fn report_verdict(report: &ValidationReport) -> &'static str {
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

pub fn report_has_failures(report: &ValidationReport) -> bool {
    report.restore_error_count != 0
        || report.mismatch_count != 0
        || report.read_error_count != 0
        || report.write_error_count != 0
        || !report.completed_all_samples
}

pub fn right_align_cell(text: &str, width: usize) -> String {
    if text.len() >= width {
        text.to_string()
    } else {
        format!("{text:>width$}")
    }
}

pub fn format_basis_points(value: u32) -> String {
    let whole = value / 100;
    let frac = value % 100;
    format!("{whole}.{frac:02}")
}

pub fn format_decimal_millis(value: u32) -> String {
    let whole = value / 1000;
    let frac = value % 1000;
    format!("{whole}.{frac:03}")
}
