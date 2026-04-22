use std::{fmt::Write as _, fs, path::Path};

use crate::{
    format_bytes, format_local_timestamp,
    model::{
        TargetInfo, TimingSeries, TimingSummary, ValidationReport, DRIVECK_MAP_COLUMNS,
        DRIVECK_MAP_ROWS, DRIVECK_SAMPLE_COUNT,
    },
    report_verdict, sample_status_glyph, sample_status_name, DriveCkError,
};

pub fn summarize_timings(series: &TimingSeries, region_size_bytes: u64) -> TimingSummary {
    let mut summary = TimingSummary::default();
    if series.values.is_empty() {
        return summary;
    }

    summary.count = series.values.len();
    summary.minimum_ms = series.values[0];
    summary.maximum_ms = series.values[0];
    summary.total_ms = series.values.iter().sum();
    summary.mean_ms = summary.total_ms / summary.count as f64;

    for &value in &series.values {
        summary.minimum_ms = summary.minimum_ms.min(value);
        summary.maximum_ms = summary.maximum_ms.max(value);
    }

    let mut sorted = series.values.clone();
    sorted.sort_by(|left, right| left.total_cmp(right));
    summary.median_ms = if sorted.len() % 2 == 0 {
        let middle = sorted.len() / 2;
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[sorted.len() / 2]
    };

    if summary.count > 1 {
        let variance = series
            .values
            .iter()
            .map(|value| {
                let delta = *value - summary.mean_ms;
                delta * delta
            })
            .sum::<f64>()
            / summary.count as f64;
        summary.stddev_ms = variance.sqrt();
    }

    if summary.mean_ms > 0.0 {
        summary.variation = summary.stddev_ms / summary.mean_ms;
    }

    if summary.total_ms > 0.0 {
        let total_bytes = region_size_bytes as f64 * summary.count as f64;
        summary.throughput_mib_s = (total_bytes / (1024.0 * 1024.0)) / (summary.total_ms / 1000.0);
    }

    summary
}

fn write_timing_block(
    output: &mut String,
    label: &str,
    series: &TimingSeries,
    region_size_bytes: u64,
) {
    let summary = summarize_timings(series, region_size_bytes);
    let _ = writeln!(output, "{label} timings:");
    if summary.count == 0 {
        let _ = writeln!(output, "  no successful {label} operations recorded");
        return;
    }

    let _ = writeln!(
        output,
        "  ops={}  min={:.3} ms  median={:.3} ms  mean={:.3} ms  max={:.3} ms  stddev={:.3} ms  variation={:.3}  total={:.3} ms  throughput={:.2} MiB/s",
        summary.count,
        summary.minimum_ms,
        summary.median_ms,
        summary.mean_ms,
        summary.maximum_ms,
        summary.stddev_ms,
        summary.variation,
        summary.total_ms,
        summary.throughput_mib_s
    );
}

fn write_map(output: &mut String, report: &ValidationReport) {
    let _ = writeln!(
        output,
        "Drive map ({}x{}, . ok, R read, W write, M mismatch, ! restore, ? untested):",
        DRIVECK_MAP_ROWS, DRIVECK_MAP_COLUMNS
    );
    for row in 0..DRIVECK_MAP_ROWS {
        let _ = write!(output, "  {row:02} ");
        for column in 0..DRIVECK_MAP_COLUMNS {
            let index = row * DRIVECK_MAP_COLUMNS + column;
            output.push(sample_status_glyph(report.sample_status[index]));
        }
        output.push('\n');
    }
}

fn write_failure_list(output: &mut String, report: &ValidationReport) {
    let mut written = 0usize;
    for index in 0..DRIVECK_SAMPLE_COUNT {
        let status = report.sample_status[index];
        if matches!(
            status,
            crate::SampleStatus::Ok | crate::SampleStatus::Untested
        ) {
            continue;
        }

        if written == 0 {
            let _ = writeln!(output, "Failures:");
        }

        let _ = writeln!(
            output,
            "  region {index:03} @ {}: {}",
            format_bytes(report.sample_offsets[index]),
            sample_status_name(status)
        );
        written += 1;
        if written == 12 {
            let remaining = report.read_error_count
                + report.write_error_count
                + report.mismatch_count
                + report.restore_error_count
                - written;
            if remaining > 0 {
                let _ = writeln!(output, "  ... {remaining} more omitted");
            }
            break;
        }
    }

    if written == 0 {
        let _ = writeln!(output, "Failures:");
        let _ = writeln!(output, "  none");
    }
}

pub fn format_report_text(target: &TargetInfo, report: &ValidationReport) -> String {
    let mut output = String::new();

    let _ = writeln!(output, "DriveCk");
    let _ = writeln!(output, "=======");
    let _ = writeln!(output, "Target:    {}", target.path);
    let _ = writeln!(output, "Kind:      block device");

    let transport_suffix = match (target.is_usb, target.is_removable) {
        (true, true) => " (usb, removable)",
        (true, false) => " (usb)",
        (false, true) => " (removable)",
        (false, false) => "",
    };
    let _ = writeln!(
        output,
        "Transport: {}{}",
        if target.transport.is_empty() {
            "-"
        } else {
            target.transport.as_str()
        },
        transport_suffix
    );
    if !target.vendor.is_empty() || !target.model.is_empty() {
        let _ = writeln!(
            output,
            "Model:     {}{}{}",
            target.vendor,
            if !target.vendor.is_empty() && !target.model.is_empty() {
                " "
            } else {
                ""
            },
            target.model
        );
    }
    let _ = writeln!(
        output,
        "Started:   {}",
        format_local_timestamp(report.started_at)
    );
    let _ = writeln!(
        output,
        "Finished:  {}",
        format_local_timestamp(report.finished_at)
    );
    let _ = writeln!(output, "Seed:      0x{:016x}", report.seed);
    let _ = writeln!(output, "Verdict:   {}", report_verdict(report));
    output.push('\n');

    let _ = writeln!(output, "Validation summary:");
    let _ = writeln!(
        output,
        "  declared size:           {} ({} bytes)",
        format_bytes(report.reported_size_bytes),
        report.reported_size_bytes
    );
    let _ = writeln!(
        output,
        "  validated drive size:    {} ({} bytes)",
        format_bytes(report.validated_drive_size_bytes),
        report.validated_drive_size_bytes
    );
    let _ = writeln!(
        output,
        "  highest valid region:    {} ({} bytes)",
        format_bytes(report.highest_valid_region_bytes),
        report.highest_valid_region_bytes
    );
    let _ = writeln!(
        output,
        "  region size:             {} ({} bytes)",
        format_bytes(report.region_size_bytes),
        report.region_size_bytes
    );
    let _ = writeln!(
        output,
        "  samples processed:       {} / {}",
        report.completed_samples, DRIVECK_SAMPLE_COUNT
    );
    let _ = writeln!(
        output,
        "  all samples completed:   {}",
        if report.completed_all_samples {
            "yes"
        } else {
            "no"
        }
    );
    let _ = writeln!(
        output,
        "  cancelled:               {}",
        if report.cancelled { "yes" } else { "no" }
    );
    let _ = writeln!(
        output,
        "  samples ok:              {} / {}",
        report.success_count, DRIVECK_SAMPLE_COUNT
    );
    let _ = writeln!(
        output,
        "  read errors:             {}",
        report.read_error_count
    );
    let _ = writeln!(
        output,
        "  write errors:            {}",
        report.write_error_count
    );
    let _ = writeln!(
        output,
        "  verify mismatches:       {}",
        report.mismatch_count
    );
    let _ = writeln!(
        output,
        "  restore errors:          {}",
        report.restore_error_count
    );
    output.push('\n');

    write_timing_block(
        &mut output,
        "Read",
        &report.read_timings,
        report.region_size_bytes,
    );
    write_timing_block(
        &mut output,
        "Write",
        &report.write_timings,
        report.region_size_bytes,
    );
    output.push('\n');

    write_map(&mut output, report);
    output.push('\n');
    write_failure_list(&mut output, report);
    output
}

pub fn save_report(
    path: impl AsRef<Path>,
    target: &TargetInfo,
    report: &ValidationReport,
) -> Result<(), DriveCkError> {
    fs::write(path.as_ref(), format_report_text(target, report)).map_err(|error| {
        DriveCkError::io(
            format!("Failed to write report {}", path.as_ref().display()),
            error,
        )
    })
}

#[cfg(test)]
mod tests {
    use crate::{
        SampleStatus, TargetInfo, TargetKind, ValidationReport, DRIVECK_MAP_COLUMNS,
        DRIVECK_SAMPLE_COUNT,
    };

    #[test]
    fn formats_report_map() {
        let mut report = ValidationReport::default();
        report.started_at = 1;
        report.finished_at = 2;
        report.seed = 3;
        report.region_size_bytes = 4096;
        report.reported_size_bytes = 4096 * DRIVECK_SAMPLE_COUNT as u64;
        report.completed_samples = DRIVECK_SAMPLE_COUNT;
        report.completed_all_samples = true;
        report.success_count = DRIVECK_SAMPLE_COUNT;
        report.sample_status.fill(SampleStatus::Ok);
        let target = TargetInfo {
            kind: TargetKind::BlockDevice,
            path: "/dev/sdz".into(),
            name: "sdz".into(),
            transport: "usb".into(),
            is_block_device: true,
            logical_block_size: 4096,
            ..TargetInfo::default()
        };

        let text = super::format_report_text(&target, &report);
        assert!(text.contains("Drive map (18x32"));
        assert!(text.contains(&format!("00 {}", ".".repeat(DRIVECK_MAP_COLUMNS))));
    }
}
