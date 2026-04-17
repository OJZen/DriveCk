use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader},
    os::fd::AsRawFd,
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use libc::dev_t;

use crate::{DriveCkError, TargetInfo, TargetKind};

pub(crate) struct OpenedTarget {
    file: File,
    direct_io_used: bool,
}

impl OpenedTarget {
    pub(crate) fn open(target: &TargetInfo) -> Result<Self, DriveCkError> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).custom_flags(libc::O_CLOEXEC);

        if target.is_block_device {
            options.custom_flags(libc::O_CLOEXEC | libc::O_DIRECT | libc::O_SYNC);
            let file = options.open(&target.path).map_err(|error| {
                DriveCkError::io(
                    format!(
                        "Failed to open {} with direct block-device I/O",
                        target.path
                    ),
                    error,
                )
            })?;
            return Ok(Self {
                file,
                direct_io_used: true,
            });
        }

        let file = options
            .open(&target.path)
            .map_err(|error| DriveCkError::io(format!("Failed to open {}", target.path), error))?;
        Ok(Self {
            file,
            direct_io_used: false,
        })
    }

    pub(crate) fn direct_io_used(&self) -> bool {
        self.direct_io_used
    }

    pub(crate) fn read_exact_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<()> {
        positioned_read(self.file.as_raw_fd(), buffer, offset)
    }

    pub(crate) fn write_all_at(&self, buffer: &[u8], offset: u64) -> io::Result<()> {
        positioned_write(self.file.as_raw_fd(), buffer, offset)
    }

    pub(crate) fn flush_target(&self, flush_required: bool) -> io::Result<()> {
        if !flush_required {
            return Ok(());
        }

        let fd = self.file.as_raw_fd();
        let result = unsafe { libc::fdatasync(fd) };
        if result == 0 {
            return Ok(());
        }

        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(code)
                if code == libc::EINVAL || code == libc::ENOTSUP || code == libc::EOPNOTSUPP =>
            {
                let fallback = unsafe { libc::fsync(fd) };
                if fallback == 0 {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error())
                }
            }
            _ => Err(error),
        }
    }

    pub(crate) fn drop_cache(&self, offset: u64, size: usize, drop_required: bool) {
        if !drop_required {
            return;
        }

        unsafe {
            let _ = libc::posix_fadvise(
                self.file.as_raw_fd(),
                offset as libc::off_t,
                size as libc::off_t,
                libc::POSIX_FADV_DONTNEED,
            );
        }
    }
}

pub(crate) fn collect_targets() -> Result<Vec<TargetInfo>, DriveCkError> {
    let mut targets = Vec::new();
    for entry in fs::read_dir("/sys/block")
        .map_err(|error| DriveCkError::io("Failed to open /sys/block", error))?
    {
        let entry = entry.map_err(|error| DriveCkError::io("Failed to read /sys/block", error))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let path = format!("/dev/{name}");
        let target = match fill_block_target(&name, &path, false) {
            Ok(target) => target,
            Err(_) => continue,
        };
        if target.is_usb || target.is_removable {
            targets.push(target);
        }
    }

    targets.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(targets)
}

pub(crate) fn discover_target(path: &Path) -> Result<TargetInfo, DriveCkError> {
    let metadata = fs::metadata(path)
        .map_err(|error| DriveCkError::io(format!("Cannot stat {}", path.display()), error))?;
    let file_type = metadata.file_type();

    if file_type.is_block_device() {
        let device_name = basename(path).ok_or_else(|| {
            DriveCkError::new(format!(
                "Cannot determine the device name for {}.",
                path.display()
            ))
        })?;
        return fill_block_target(&device_name, &path.to_string_lossy(), true);
    }

    Err(DriveCkError::new(format!(
        "Target {} is not a whole block device.",
        path.display()
    )))
}

fn positioned_read(fd: i32, buffer: &mut [u8], mut offset: u64) -> io::Result<()> {
    let mut cursor = 0usize;
    while cursor < buffer.len() {
        let result = unsafe {
            libc::pread(
                fd,
                buffer[cursor..].as_mut_ptr().cast(),
                buffer.len() - cursor,
                offset as libc::off_t,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if result == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short positioned read",
            ));
        }

        cursor += result as usize;
        offset += result as u64;
    }
    Ok(())
}

fn positioned_write(fd: i32, buffer: &[u8], mut offset: u64) -> io::Result<()> {
    let mut cursor = 0usize;
    while cursor < buffer.len() {
        let result = unsafe {
            libc::pwrite(
                fd,
                buffer[cursor..].as_ptr().cast(),
                buffer.len() - cursor,
                offset as libc::off_t,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if result == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short positioned write",
            ));
        }

        cursor += result as usize;
        offset += result as u64;
    }
    Ok(())
}

fn fill_block_target(
    device_name: &str,
    path: &str,
    reject_mounted: bool,
) -> Result<TargetInfo, DriveCkError> {
    let sysfs_root = PathBuf::from(format!("/sys/class/block/{device_name}"));
    if !sysfs_root.exists() {
        return Err(DriveCkError::new(format!(
            "Block device {} is not exposed through /sys/class/block.",
            path
        )));
    }

    if sysfs_root.join("partition").exists() {
        return Err(DriveCkError::new(format!(
            "Target {} is a partition. Pass the whole-disk node instead (for example /dev/{}).",
            path, device_name
        )));
    }

    let mounted = is_block_device_mounted(device_name);
    if reject_mounted && mounted {
        return Err(DriveCkError::new(format!(
            "Refusing to validate {} because the disk or one of its partitions is mounted.",
            path
        )));
    }

    let sectors = read_u64_file(&sysfs_root.join("size"))
        .ok_or_else(|| DriveCkError::new(format!("Failed to read capacity for {}.", path)))?;
    let logical_block_size = read_u64_file(&sysfs_root.join("queue/logical_block_size"))
        .unwrap_or(4096)
        .max(1) as u32;
    let is_removable = read_u64_file(&sysfs_root.join("removable")).unwrap_or(0) != 0;
    let vendor = read_text_file(&sysfs_root.join("device/vendor")).unwrap_or_default();
    let model = read_text_file(&sysfs_root.join("device/model")).unwrap_or_default();
    let resolved = fs::canonicalize(&sysfs_root).ok();
    let is_usb = resolved
        .as_ref()
        .is_some_and(|path| path.to_string_lossy().contains("/usb"));
    let transport = if is_usb {
        "usb"
    } else if is_removable {
        "removable"
    } else {
        "block"
    };

    Ok(TargetInfo {
        kind: TargetKind::BlockDevice,
        path: path.to_string(),
        name: device_name.to_string(),
        vendor,
        model,
        transport: transport.to_string(),
        size_bytes: sectors * 512,
        logical_block_size,
        is_block_device: true,
        is_removable,
        is_usb,
        is_mounted: mounted,
        direct_io: true,
    })
}

fn is_block_device_mounted(device_name: &str) -> bool {
    let Ok((devices, holders_present)) = collect_block_devices(device_name) else {
        return false;
    };
    if holders_present {
        return true;
    }

    if let Ok(file) = File::open("/proc/self/mounts") {
        let reader = BufReader::new(file);
        for line in reader.lines().map_while(Result::ok) {
            let mut fields = line.split_whitespace();
            if let Some(source) = fields.next() {
                if source_matches_devices(source, &devices) {
                    return true;
                }
            }
        }
    }

    swaps_match_devices(&devices)
}

fn collect_block_devices(device_name: &str) -> Result<(Vec<dev_t>, bool), DriveCkError> {
    let sysfs_root = PathBuf::from(format!("/sys/class/block/{device_name}"));
    let mut devices = Vec::new();
    let mut holders_present = directory_has_entries(&sysfs_root.join("holders"));

    let root_device = read_dev_file(&sysfs_root.join("dev")).ok_or_else(|| {
        DriveCkError::new(format!("Failed to read device identity for {device_name}."))
    })?;
    push_unique(&mut devices, root_device);

    for entry in fs::read_dir(&sysfs_root).map_err(|error| {
        DriveCkError::io(format!("Failed to inspect {}", sysfs_root.display()), error)
    })? {
        let entry =
            entry.map_err(|error| DriveCkError::io("Failed to enumerate sysfs entry", error))?;
        let entry_path = entry.path();
        if !entry_path.join("partition").exists() {
            continue;
        }
        if let Some(dev_value) = read_dev_file(&entry_path.join("dev")) {
            push_unique(&mut devices, dev_value);
        }
        holders_present |= directory_has_entries(&entry_path.join("holders"));
    }

    Ok((devices, holders_present))
}

fn source_matches_devices(path: &str, devices: &[dev_t]) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    metadata.file_type().is_block_device() && devices.contains(&(metadata.rdev() as dev_t))
}

fn swaps_match_devices(devices: &[dev_t]) -> bool {
    let Ok(file) = File::open("/proc/swaps") else {
        return false;
    };
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return false;
    }

    for line in reader.lines().map_while(Result::ok) {
        if let Some(source) = line.split_whitespace().next() {
            if source_matches_devices(source, devices) {
                return true;
            }
        }
    }
    false
}

fn read_text_file(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    Some(text.trim_end().to_string())
}

fn read_u64_file(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn read_dev_file(path: &Path) -> Option<dev_t> {
    let text = fs::read_to_string(path).ok()?;
    let mut parts = text.trim().split(':');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let major = u32::try_from(major).ok()?;
    let minor = u32::try_from(minor).ok()?;
    Some(libc::makedev(major, minor))
}

fn directory_has_entries(path: &Path) -> bool {
    fs::read_dir(path)
        .ok()
        .and_then(|mut entries| entries.next().transpose().ok().flatten())
        .is_some()
}

fn push_unique(devices: &mut Vec<dev_t>, value: dev_t) {
    if !devices.contains(&value) {
        devices.push(value);
    }
}

fn basename(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(OsStr::to_str)
        .map(ToOwned::to_owned)
}
