use serde::{Deserialize, Serialize};

pub const DRIVECK_SAMPLE_COUNT: usize = 576;
pub const DRIVECK_MIN_REGION_SIZE: u64 = 4096;
pub const DRIVECK_MAX_REGION_SIZE: u64 = 8 * 1024 * 1024;
pub const DRIVECK_TIMING_CAPACITY: usize = DRIVECK_SAMPLE_COUNT * 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetKind {
    BlockDevice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SampleStatus {
    #[default]
    Untested,
    Ok,
    ReadError,
    WriteError,
    VerifyMismatch,
    RestoreError,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ValidationOptions {
    pub seed: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TargetInfo {
    pub kind: TargetKind,
    pub path: String,
    pub name: String,
    pub vendor: String,
    pub model: String,
    pub transport: String,
    pub size_bytes: u64,
    pub logical_block_size: u32,
    pub is_block_device: bool,
    pub is_removable: bool,
    pub is_usb: bool,
    pub is_mounted: bool,
    pub direct_io: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TimingSeries {
    pub values: Vec<f64>,
}

impl TimingSeries {
    pub fn with_capacity() -> Self {
        Self {
            values: Vec::with_capacity(DRIVECK_TIMING_CAPACITY),
        }
    }

    pub fn push(&mut self, elapsed_ms: f64) {
        if self.values.len() < DRIVECK_TIMING_CAPACITY {
            self.values.push(elapsed_ms);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TimingSummary {
    pub count: usize,
    pub minimum_ms: f64,
    pub median_ms: f64,
    pub mean_ms: f64,
    pub maximum_ms: f64,
    pub stddev_ms: f64,
    pub total_ms: f64,
    pub variation: f64,
    pub throughput_mib_s: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub started_at: i64,
    pub finished_at: i64,
    pub seed: u64,
    pub reported_size_bytes: u64,
    pub region_size_bytes: u64,
    pub validated_drive_size_bytes: u64,
    pub highest_valid_region_bytes: u64,
    pub sample_offsets: Vec<u64>,
    pub sample_status: Vec<SampleStatus>,
    pub read_timings: TimingSeries,
    pub write_timings: TimingSeries,
    pub success_count: usize,
    pub read_error_count: usize,
    pub write_error_count: usize,
    pub mismatch_count: usize,
    pub restore_error_count: usize,
    pub completed_samples: usize,
    pub cancelled: bool,
    pub completed_all_samples: bool,
}

impl Default for ValidationReport {
    fn default() -> Self {
        Self {
            started_at: 0,
            finished_at: 0,
            seed: 0,
            reported_size_bytes: 0,
            region_size_bytes: 0,
            validated_drive_size_bytes: 0,
            highest_valid_region_bytes: 0,
            sample_offsets: vec![0; DRIVECK_SAMPLE_COUNT],
            sample_status: vec![SampleStatus::Untested; DRIVECK_SAMPLE_COUNT],
            read_timings: TimingSeries::with_capacity(),
            write_timings: TimingSeries::with_capacity(),
            success_count: 0,
            read_error_count: 0,
            write_error_count: 0,
            mismatch_count: 0,
            restore_error_count: 0,
            completed_samples: 0,
            cancelled: false,
            completed_all_samples: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResponse {
    pub target: TargetInfo,
    pub report: ValidationReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressUpdate {
    pub phase: &'static str,
    pub current: usize,
    pub total: usize,
    pub final_update: bool,
}

pub trait ProgressObserver {
    fn on_progress(&mut self, update: ProgressUpdate);
}

impl<F> ProgressObserver for F
where
    F: FnMut(ProgressUpdate),
{
    fn on_progress(&mut self, update: ProgressUpdate) {
        self(update);
    }
}

pub trait CancelObserver {
    fn is_cancelled(&self) -> bool;
}

impl<F> CancelObserver for F
where
    F: Fn() -> bool,
{
    fn is_cancelled(&self) -> bool {
        self()
    }
}

impl Default for TargetKind {
    fn default() -> Self {
        Self::BlockDevice
    }
}
