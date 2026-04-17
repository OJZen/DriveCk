#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

use std::path::Path;

use crate::{DriveCkError, TargetInfo};

#[cfg(target_os = "linux")]
pub(crate) use linux::OpenedTarget;
#[cfg(windows)]
pub(crate) use windows::OpenedTarget;

pub(crate) fn collect_targets() -> Result<Vec<TargetInfo>, DriveCkError> {
    #[cfg(target_os = "linux")]
    {
        return linux::collect_targets();
    }
    #[cfg(windows)]
    {
        return windows::collect_targets();
    }
    #[allow(unreachable_code)]
    Err(DriveCkError::new(
        "Target discovery is not implemented on this platform.",
    ))
}

pub(crate) fn discover_target(path: &Path) -> Result<TargetInfo, DriveCkError> {
    #[cfg(target_os = "linux")]
    {
        return linux::discover_target(path);
    }
    #[cfg(windows)]
    {
        return windows::discover_target(path);
    }
    #[allow(unreachable_code)]
    Err(DriveCkError::new(format!(
        "Target discovery is not implemented for {}.",
        path.display()
    )))
}
