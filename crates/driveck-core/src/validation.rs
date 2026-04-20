use std::{
    alloc::{Layout, alloc_zeroed, dealloc},
    fmt,
    mem::size_of,
    ptr::NonNull,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    CancelObserver, DRIVECK_MIN_REGION_SIZE, DRIVECK_SAMPLE_COUNT, ProgressObserver,
    ProgressUpdate, SampleStatus, TargetInfo, TimingSeries, ValidationOptions, ValidationReport,
    format_bytes, platform::OpenedTarget,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationFailure {
    pub message: String,
    pub report: Option<ValidationReport>,
}

impl ValidationFailure {
    fn new(message: impl Into<String>, report: Option<ValidationReport>) -> Self {
        Self {
            message: message.into(),
            report,
        }
    }
}

impl fmt::Display for ValidationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ValidationFailure {}

#[derive(Clone, Copy)]
struct DriveCkRng {
    x: u32,
    y: u32,
    z: u32,
    w: u32,
}

struct AlignedBuffer {
    ptr: NonNull<u8>,
    len: usize,
    align: usize,
}

struct ProbeBuffers {
    original: AlignedBuffer,
    pattern: AlignedBuffer,
    readback: AlignedBuffer,
}

unsafe impl Send for AlignedBuffer {}

impl AlignedBuffer {
    fn new(align: usize, len: usize) -> Result<Self, ValidationFailure> {
        let layout = Layout::from_size_align(len.max(1), align).map_err(|_| {
            ValidationFailure::new(
                format!("Failed to allocate {len}-byte aligned validation buffer."),
                None,
            )
        })?;
        let raw = unsafe { alloc_zeroed(layout) };
        let ptr = NonNull::new(raw).ok_or_else(|| {
            ValidationFailure::new(
                format!("Failed to allocate {len}-byte aligned validation buffer."),
                None,
            )
        })?;
        Ok(Self { ptr, len, align })
    }

    fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        let layout = Layout::from_size_align(self.len.max(1), self.align)
            .expect("valid aligned buffer layout");
        unsafe {
            dealloc(self.ptr.as_ptr(), layout);
        }
    }
}

impl ProbeBuffers {
    fn new(alignment: usize, size: usize) -> Result<Self, ValidationFailure> {
        Ok(Self {
            original: AlignedBuffer::new(alignment, size)?,
            pattern: AlignedBuffer::new(alignment, size)?,
            readback: AlignedBuffer::new(alignment, size)?,
        })
    }
}

pub fn validate_target(
    target: &TargetInfo,
    options: &ValidationOptions,
) -> Result<ValidationReport, ValidationFailure> {
    validate_target_with_callbacks(target, options, None, None)
}

pub fn validate_target_with_callbacks(
    target: &TargetInfo,
    options: &ValidationOptions,
    mut progress: Option<&mut dyn ProgressObserver>,
    cancel: Option<&dyn CancelObserver>,
) -> Result<ValidationReport, ValidationFailure> {
    if !target.is_block_device {
        return Err(ValidationFailure::new(
            "DriveCk only validates whole block devices. Non-device paths are not supported.",
            None,
        ));
    }
    if target.is_mounted {
        return Err(ValidationFailure::new(
            format!(
                "Refusing to validate {} because the disk or one of its volumes is mounted.",
                target.path
            ),
            None,
        ));
    }

    let mut report = ValidationReport::default();
    report.started_at = current_timestamp();
    report.reported_size_bytes = target.size_bytes;
    report.seed = options.seed.unwrap_or_else(|| default_seed(target));

    let alignment = DRIVECK_MIN_REGION_SIZE.max(u64::from(target.logical_block_size.max(4096)));
    let opened =
        OpenedTarget::open(target).map_err(|error| ValidationFailure::new(error.message, None))?;
    let direct_io_active = effective_direct_io(opened.direct_io_used());
    let region_size = default_region_size(alignment, target.size_bytes).map_err(|message| {
        report.finished_at = current_timestamp();
        ValidationFailure::new(message, Some(report.clone()))
    })?;

    report.region_size_bytes = region_size as u64;
    for index in 0..DRIVECK_SAMPLE_COUNT {
        report.sample_offsets[index] = sample_offset(
            target.size_bytes,
            report.region_size_bytes,
            alignment,
            index,
        );
    }

    let mut buffers = ProbeBuffers::new(alignment as usize, region_size).map_err(|error| {
        ValidationFailure::new(
            error.message,
            Some(ValidationReport {
                region_size_bytes: report.region_size_bytes,
                ..report.clone()
            }),
        )
    })?;

    let order = build_sample_order(report.seed);
    for sample_index in order {
        if is_cancelled(cancel) {
            report.cancelled = true;
            break;
        }

        let offset = report.sample_offsets[sample_index];
        let status = {
            let mut read_timings = Some(&mut report.read_timings);
            let mut write_timings = Some(&mut report.write_timings);
            probe_region(
                &opened,
                direct_io_active,
                offset,
                region_size,
                report.seed,
                sample_index,
                &mut buffers,
                &mut read_timings,
                &mut write_timings,
            )
        };
        report.sample_status[sample_index] = status;
        count_status(&mut report, status);
        report.completed_samples += 1;
        emit_progress(
            &mut progress,
            "Validating",
            report.completed_samples,
            DRIVECK_SAMPLE_COUNT,
            Some(sample_index),
            Some(status),
            false,
        );

        if status == SampleStatus::RestoreError {
            break;
        }
    }

    emit_progress(
        &mut progress,
        "Validating",
        report.completed_samples,
        DRIVECK_SAMPLE_COUNT,
        None,
        None,
        true,
    );
    report.completed_all_samples =
        !report.cancelled && report.completed_samples == DRIVECK_SAMPLE_COUNT;
    report.finished_at = current_timestamp();
    finalize_extents(&mut report);

    if report.cancelled {
        return Err(ValidationFailure::new(
            "Validation cancelled.",
            Some(report),
        ));
    }

    Ok(report)
}

pub fn build_sample_order(seed: u64) -> [usize; DRIVECK_SAMPLE_COUNT] {
    let mut order = [0usize; DRIVECK_SAMPLE_COUNT];
    let mut middle = [0usize; DRIVECK_SAMPLE_COUNT - 2];

    order[0] = DRIVECK_SAMPLE_COUNT - 1;
    order[1] = 0;
    for (index, slot) in middle.iter_mut().enumerate() {
        *slot = index + 1;
    }

    let mut rng = seed_rng(seed, 0x240, 0x1000);
    let mut count = middle.len();
    while count > 1 {
        let current = count - 1;
        let swap_index = (xorshift128(&mut rng) as usize) % count;
        middle.swap(current, swap_index);
        count = current;
    }

    for (index, sample_index) in middle.into_iter().enumerate() {
        order[index + 2] = sample_index;
    }

    order
}

fn emit_progress(
    progress: &mut Option<&mut dyn ProgressObserver>,
    phase: &'static str,
    current: usize,
    total: usize,
    sample_index: Option<usize>,
    sample_status: Option<SampleStatus>,
    final_update: bool,
) {
    if let Some(observer) = progress.as_deref_mut() {
        observer.on_progress(ProgressUpdate {
            phase,
            current,
            total,
            sample_index,
            sample_status,
            final_update,
        });
    }
}

fn is_cancelled(cancel: Option<&dyn CancelObserver>) -> bool {
    cancel.is_some_and(|token| token.is_cancelled())
}

fn default_region_size(alignment: u64, target_size: u64) -> Result<usize, String> {
    let minimum_region = DRIVECK_MIN_REGION_SIZE.max(alignment).next_power_of_two();
    if target_size < minimum_region {
        return Err(format!(
            "Target is too small for a {} validation region.",
            format_bytes(minimum_region)
        ));
    }
    Ok(minimum_region as usize)
}

fn effective_direct_io(opened_direct_io: bool) -> bool {
    opened_direct_io
}

fn recorded_read(
    opened: &OpenedTarget,
    offset: u64,
    buffer: &mut [u8],
    series: &mut Option<&mut TimingSeries>,
    drop_cache_before_read: bool,
) -> std::io::Result<()> {
    opened.drop_cache(offset, buffer.len(), drop_cache_before_read);
    let started = Instant::now();
    opened.read_exact_at(buffer, offset)?;
    if let Some(series) = series.as_deref_mut() {
        series.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(())
}

fn recorded_write(
    opened: &OpenedTarget,
    offset: u64,
    buffer: &[u8],
    series: &mut Option<&mut TimingSeries>,
    flush_after_write: bool,
) -> std::io::Result<()> {
    let started = Instant::now();
    opened.write_all_at(buffer, offset)?;
    opened.flush_target(flush_after_write)?;
    if let Some(series) = series.as_deref_mut() {
        series.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(())
}

fn probe_region(
    opened: &OpenedTarget,
    direct_io_active: bool,
    offset: u64,
    size: usize,
    seed: u64,
    sample_index: usize,
    buffers: &mut ProbeBuffers,
    read_timings: &mut Option<&mut TimingSeries>,
    write_timings: &mut Option<&mut TimingSeries>,
) -> SampleStatus {
    let drop_cache_before_read = !direct_io_active;
    let flush_after_write = !direct_io_active;

    if recorded_read(
        opened,
        offset,
        buffers.original.as_mut_slice(),
        read_timings,
        drop_cache_before_read,
    )
    .is_err()
    {
        return SampleStatus::ReadError;
    }

    fill_pattern(
        buffers.pattern.as_mut_slice(),
        seed,
        sample_index as u64,
        offset,
    );

    if recorded_write(
        opened,
        offset,
        buffers.pattern.as_slice(),
        write_timings,
        flush_after_write,
    )
    .is_err()
    {
        if recorded_write(
            opened,
            offset,
            buffers.original.as_slice(),
            write_timings,
            flush_after_write,
        )
        .is_err()
        {
            return SampleStatus::RestoreError;
        }
        return SampleStatus::WriteError;
    }

    if recorded_read(
        opened,
        offset,
        buffers.readback.as_mut_slice(),
        read_timings,
        drop_cache_before_read,
    )
    .is_err()
    {
        if recorded_write(
            opened,
            offset,
            buffers.original.as_slice(),
            write_timings,
            flush_after_write,
        )
        .is_err()
        {
            return SampleStatus::RestoreError;
        }
        return SampleStatus::ReadError;
    }

    if buffers.pattern.as_slice()[..size] != buffers.readback.as_slice()[..size] {
        if recorded_write(
            opened,
            offset,
            buffers.original.as_slice(),
            write_timings,
            flush_after_write,
        )
        .is_err()
        {
            return SampleStatus::RestoreError;
        }
        return SampleStatus::VerifyMismatch;
    }

    if recorded_write(
        opened,
        offset,
        buffers.original.as_slice(),
        write_timings,
        flush_after_write,
    )
    .is_err()
    {
        return SampleStatus::RestoreError;
    }

    SampleStatus::Ok
}

fn fill_pattern(buffer: &mut [u8], seed: u64, sample_index: u64, offset: u64) {
    let mut rng = seed_rng(seed, sample_index, offset);
    let mut cursor = 0usize;
    while cursor + size_of::<u32>() <= buffer.len() {
        let value = xorshift128(&mut rng).to_le_bytes();
        buffer[cursor..cursor + 4].copy_from_slice(&value);
        cursor += 4;
    }
    if cursor < buffer.len() {
        let value = xorshift128(&mut rng).to_le_bytes();
        let remaining = buffer.len() - cursor;
        buffer[cursor..].copy_from_slice(&value[..remaining]);
    }
}

fn seed_rng(seed: u64, sample_index: u64, offset: u64) -> DriveCkRng {
    let mixed = seed
        ^ sample_index.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ offset.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    let mut rng = DriveCkRng {
        x: mixed as u32,
        y: (mixed >> 32) as u32,
        z: (mixed as u32) ^ 0xA341_316C,
        w: ((mixed >> 32) as u32) ^ 0xC801_3EA4,
    };
    if (rng.x | rng.y | rng.z | rng.w) == 0 {
        rng.w = 1;
    }
    rng
}

fn xorshift128(rng: &mut DriveCkRng) -> u32 {
    let t = rng.x ^ (rng.x << 11);
    rng.x = rng.y;
    rng.y = rng.z;
    rng.z = rng.w;
    rng.w ^= rng.w >> 19;
    rng.w ^= t;
    rng.w ^= t >> 8;
    rng.w
}

fn default_seed(target: &TargetInfo) -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    hash_path(&target.path)
        ^ target.size_bytes
        ^ now.as_secs()
        ^ ((u64::from(now.subsec_nanos())) << 17)
}

fn hash_path(path: &str) -> u64 {
    let mut hash = 1469598103934665603u64;
    for byte in path.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(1099511628211);
    }
    hash
}

fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn align_down(value: u64, alignment: u64) -> u64 {
    if alignment == 0 {
        value
    } else {
        value - (value % alignment)
    }
}

fn sample_offset(target_size: u64, region_size: u64, alignment: u64, sample_index: usize) -> u64 {
    if target_size <= region_size {
        return 0;
    }

    let max_start = target_size - region_size;
    if sample_index + 1 >= DRIVECK_SAMPLE_COUNT {
        return align_down(max_start, alignment);
    }

    let denominator = (DRIVECK_SAMPLE_COUNT - 1) as u128;
    let numerator = max_start as u128 * sample_index as u128;
    let rounded = ((numerator + denominator / 2) / denominator) as u64;
    let candidate = align_down(rounded, alignment);
    candidate.min(align_down(max_start, alignment))
}

fn sample_end(report: &ValidationReport, sample_index: usize) -> u64 {
    if sample_index + 1 >= DRIVECK_SAMPLE_COUNT {
        return report.reported_size_bytes;
    }

    (report.sample_offsets[sample_index] + report.region_size_bytes).min(report.reported_size_bytes)
}

fn finalize_extents(report: &mut ValidationReport) {
    report.validated_drive_size_bytes = 0;
    report.highest_valid_region_bytes = 0;

    let mut first_non_green = None;
    for index in 0..DRIVECK_SAMPLE_COUNT {
        if report.sample_status[index] != SampleStatus::Ok {
            first_non_green = Some(index);
            break;
        }
    }

    match first_non_green {
        None => report.validated_drive_size_bytes = report.reported_size_bytes,
        Some(index) if report.sample_status[index] != SampleStatus::Untested => {
            report.validated_drive_size_bytes = sample_end(report, index);
        }
        Some(index) if index > 0 => {
            report.validated_drive_size_bytes = sample_end(report, index - 1);
        }
        _ => {}
    }

    for index in (0..DRIVECK_SAMPLE_COUNT).rev() {
        if report.sample_status[index] == SampleStatus::Ok {
            report.highest_valid_region_bytes = sample_end(report, index);
            break;
        }
    }
}

fn count_status(report: &mut ValidationReport, status: SampleStatus) {
    match status {
        SampleStatus::Ok => report.success_count += 1,
        SampleStatus::ReadError => report.read_error_count += 1,
        SampleStatus::WriteError => report.write_error_count += 1,
        SampleStatus::VerifyMismatch => report.mismatch_count += 1,
        SampleStatus::RestoreError => report.restore_error_count += 1,
        SampleStatus::Untested => {}
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        DRIVECK_MIN_REGION_SIZE, DRIVECK_SAMPLE_COUNT, ValidationOptions, validate_target,
    };

    use super::{TargetInfo, build_sample_order, default_region_size, effective_direct_io};

    #[test]
    fn sample_order_preserves_front_and_back_priority() {
        let order = build_sample_order(0x1234);
        assert_eq!(order[0], 575);
        assert_eq!(order[1], 0);

        let mut sorted = order.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, (0usize..576).collect::<Vec<_>>());
    }

    #[test]
    fn default_region_size_uses_4k_floor() {
        assert_eq!(
            default_region_size(512, DRIVECK_MIN_REGION_SIZE * DRIVECK_SAMPLE_COUNT as u64)
                .expect("4 KB floor should be valid"),
            DRIVECK_MIN_REGION_SIZE as usize
        );
    }

    #[test]
    fn default_region_size_respects_large_block_sizes() {
        assert_eq!(
            default_region_size(8192, 8192 * DRIVECK_SAMPLE_COUNT as u64)
                .expect("aligned default region should be valid"),
            8192
        );
    }

    #[test]
    fn only_confirmed_direct_io_enables_fast_path() {
        assert!(effective_direct_io(true));
        assert!(!effective_direct_io(false));
    }

    #[test]
    fn mounted_targets_are_rejected_before_opening() {
        let target = TargetInfo {
            path: "/path/that/should/not/be/opened".into(),
            name: "diskX".into(),
            size_bytes: DRIVECK_MIN_REGION_SIZE * DRIVECK_SAMPLE_COUNT as u64,
            logical_block_size: DRIVECK_MIN_REGION_SIZE as u32,
            is_block_device: true,
            is_mounted: true,
            ..TargetInfo::default()
        };

        let error = validate_target(&target, &ValidationOptions::default())
            .expect_err("mounted targets must be rejected");
        assert!(error.message.contains("mounted"));
    }
}
