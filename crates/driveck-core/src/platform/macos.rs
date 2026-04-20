use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io,
    os::fd::AsRawFd,
    os::unix::fs::{FileTypeExt, OpenOptionsExt},
    path::Path,
};

use crate::{DriveCkError, TargetInfo};

pub(crate) struct OpenedTarget {
    file: File,
    direct_io_used: bool,
}

impl OpenedTarget {
    pub(crate) fn open(target: &TargetInfo) -> Result<Self, DriveCkError> {
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            // We already issue explicit durability barriers through flush_target() when the
            // validation path requires them. Keeping O_SYNC here forces every pwrite() to
            // synchronously drain on top of those barriers and makes the macOS fast path
            // effectively unreachable in practice.
            .custom_flags(libc::O_CLOEXEC | libc::O_EXLOCK | libc::O_NONBLOCK);

        let file = options
            .open(&target.path)
            .map_err(|error| map_open_error(target, error))?;
        let fd = file.as_raw_fd();
        if let Err(error) = clear_nonblocking(fd) {
            if !can_tolerate_nonblocking_configuration_error(&error) {
                return Err(DriveCkError::io(
                    format!("Failed to configure {}", target.path),
                    error,
                ));
            }
        }
        let direct_io_used = unsafe { libc::fcntl(fd, libc::F_NOCACHE, 1) } == 0;

        Ok(Self {
            file,
            direct_io_used,
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
        let result = unsafe { libc::fcntl(fd, libc::F_FULLFSYNC) };
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

    pub(crate) fn drop_cache(&self, _offset: u64, _size: usize, drop_required: bool) {
        if !drop_required {
            return;
        }

        unsafe {
            let _ = libc::fcntl(self.file.as_raw_fd(), libc::F_NOCACHE, 1);
        }
    }
}

pub(crate) fn collect_targets() -> Result<Vec<TargetInfo>, DriveCkError> {
    Err(DriveCkError::new(
        "Target discovery is implemented in the macOS layer.",
    ))
}

pub(crate) fn discover_target(path: &Path) -> Result<TargetInfo, DriveCkError> {
    let metadata = fs::metadata(path)
        .map_err(|error| DriveCkError::io(format!("Cannot stat {}", path.display()), error))?;
    let file_type = metadata.file_type();
    let device_name = basename(path).ok_or_else(|| {
        DriveCkError::new(format!(
            "Cannot determine the device name for {}.",
            path.display()
        ))
    })?;

    if !(file_type.is_char_device() || file_type.is_block_device()) {
        return Err(DriveCkError::new(format!(
            "Target {} is not a whole block device.",
            path.display()
        )));
    }

    if is_whole_disk_name(&device_name) {
        return Err(DriveCkError::new(format!(
            "Target discovery is implemented in the macOS layer for {}.",
            path.display()
        )));
    }

    if is_partition_name(&device_name) {
        return Err(DriveCkError::new(format!(
            "Target {} is a partition. Pass the whole-disk node instead.",
            path.display()
        )));
    }

    Err(DriveCkError::new(format!(
        "Target {} is not a supported whole-disk node.",
        path.display()
    )))
}

fn clear_nonblocking(fd: i32) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if flags & libc::O_NONBLOCK == 0 {
        return Ok(());
    }

    let result = unsafe { libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn can_tolerate_nonblocking_configuration_error(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(code)
            if code == libc::EINVAL
                || code == libc::ENOTTY
                || code == libc::ENOTSUP
                || code == libc::EOPNOTSUPP
    )
}

fn map_open_error(target: &TargetInfo, error: io::Error) -> DriveCkError {
    match error.raw_os_error() {
        Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN => {
            DriveCkError::new(format!(
                "Failed to acquire exclusive access to {} because another process is already using the device.",
                target.path
            ))
        }
        _ => DriveCkError::io(
            format!(
                "Failed to open {} with exclusive raw-device I/O",
                target.path
            ),
            error,
        ),
    }
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
            if error.kind() == io::ErrorKind::Interrupted
                || error.kind() == io::ErrorKind::WouldBlock
            {
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
            if error.kind() == io::ErrorKind::Interrupted
                || error.kind() == io::ErrorKind::WouldBlock
            {
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

fn basename(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(OsStr::to_str)
        .map(ToOwned::to_owned)
}

fn is_whole_disk_name(name: &str) -> bool {
    parse_disk_suffix(name)
        .map(|suffix| suffix.chars().all(|ch| ch.is_ascii_digit()))
        .unwrap_or(false)
}

fn is_partition_name(name: &str) -> bool {
    parse_disk_suffix(name).is_some_and(|suffix| {
        suffix.contains('s') && suffix.split('s').all(|part| !part.is_empty())
    })
}

fn parse_disk_suffix(name: &str) -> Option<&str> {
    name.strip_prefix("disk")
        .or_else(|| name.strip_prefix("rdisk"))
}

#[cfg(test)]
mod tests {
    use super::can_tolerate_nonblocking_configuration_error;
    use std::io;

    #[test]
    fn tolerates_known_nonblocking_configuration_errors() {
        for code in [libc::EINVAL, libc::ENOTTY, libc::ENOTSUP, libc::EOPNOTSUPP] {
            let error = io::Error::from_raw_os_error(code);
            assert!(can_tolerate_nonblocking_configuration_error(&error));
        }
    }

    #[test]
    fn rejects_unrelated_configuration_errors() {
        let error = io::Error::from_raw_os_error(libc::EACCES);
        assert!(!can_tolerate_nonblocking_configuration_error(&error));
    }
}
