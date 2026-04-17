use std::{
    fs::{File, OpenOptions},
    io,
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        fs::{FileExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            BusTypeAta as BUS_TYPE_ATA, BusTypeNvme as BUS_TYPE_NVME, BusTypeSata as BUS_TYPE_SATA,
            BusTypeScsi as BUS_TYPE_SCSI, BusTypeSd as BUS_TYPE_SD, BusTypeUsb as BUS_TYPE_USB,
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_NO_BUFFERING, FILE_FLAG_WRITE_THROUGH,
            FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        },
        System::{
            IO::DeviceIoControl,
            Ioctl::{
                GET_LENGTH_INFORMATION, IOCTL_DISK_GET_LENGTH_INFO, IOCTL_STORAGE_QUERY_PROPERTY,
                PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY,
                STORAGE_QUERY_TYPE, StorageDeviceProperty,
            },
        },
    },
    core::PCWSTR,
};

use crate::{DriveCkError, TargetInfo, TargetKind};

pub(crate) struct OpenedTarget {
    file: File,
    direct_io_used: bool,
}

impl OpenedTarget {
    pub(crate) fn open(target: &TargetInfo) -> Result<Self, DriveCkError> {
        let mut options = OpenOptions::new();
        options.read(true).write(true);

        if target.is_block_device {
            options
                .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
                .custom_flags(FILE_FLAG_NO_BUFFERING.0 | FILE_FLAG_WRITE_THROUGH.0);
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

    pub(crate) fn read_exact_at(&self, buffer: &mut [u8], mut offset: u64) -> io::Result<()> {
        let mut cursor = 0usize;
        while cursor < buffer.len() {
            let read = self.file.seek_read(&mut buffer[cursor..], offset)?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "short positioned read",
                ));
            }
            cursor += read;
            offset += read as u64;
        }
        Ok(())
    }

    pub(crate) fn write_all_at(&self, buffer: &[u8], mut offset: u64) -> io::Result<()> {
        let mut cursor = 0usize;
        while cursor < buffer.len() {
            let written = self.file.seek_write(&buffer[cursor..], offset)?;
            if written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "short positioned write",
                ));
            }
            cursor += written;
            offset += written as u64;
        }
        Ok(())
    }

    pub(crate) fn flush_target(&self, flush_required: bool) -> io::Result<()> {
        if !flush_required {
            return Ok(());
        }
        self.file.sync_data()
    }

    pub(crate) fn drop_cache(&self, _offset: u64, _size: usize, _drop_required: bool) {}
}

pub(crate) fn collect_targets() -> Result<Vec<TargetInfo>, DriveCkError> {
    let mut targets = Vec::new();
    for index in 0u32..32 {
        if let Ok(target) = query_physical_drive(index) {
            if target.is_usb || target.is_removable {
                targets.push(target);
            }
        }
    }
    Ok(targets)
}

pub(crate) fn discover_target(path: &Path) -> Result<TargetInfo, DriveCkError> {
    let path_text = path.to_string_lossy().to_string();
    if let Some(index) = parse_physical_drive_index(&path_text) {
        return query_physical_drive(index);
    }

    Err(DriveCkError::new(format!(
        "Target {} is not a physical drive path.",
        path.display()
    )))
}

fn query_physical_drive(index: u32) -> Result<TargetInfo, DriveCkError> {
    let path = format!(r"\\.\PhysicalDrive{index}");
    let handle = open_metadata_handle(&path)?;
    let capacity = query_capacity(handle)
        .ok_or_else(|| DriveCkError::new(format!("Failed to query capacity for {path}.")))?;
    let descriptor = query_descriptor(handle).unwrap_or_default();
    unsafe {
        let _ = CloseHandle(handle);
    }

    Ok(TargetInfo {
        kind: TargetKind::BlockDevice,
        path: path.clone(),
        name: format!("PhysicalDrive{index}"),
        vendor: descriptor.vendor,
        model: descriptor.model,
        transport: descriptor.transport,
        size_bytes: capacity,
        logical_block_size: 4096,
        is_block_device: true,
        is_removable: descriptor.is_removable,
        is_usb: descriptor.is_usb,
        is_mounted: false,
        direct_io: true,
    })
}

#[derive(Default)]
struct DescriptorInfo {
    vendor: String,
    model: String,
    transport: String,
    is_usb: bool,
    is_removable: bool,
}

fn open_metadata_handle(path: &str) -> Result<HANDLE, DriveCkError> {
    let wide = wide(path);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            (windows::Win32::Storage::FileSystem::FILE_GENERIC_READ
                | windows::Win32::Storage::FileSystem::FILE_GENERIC_WRITE)
                .0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .map_err(|_| DriveCkError::io(format!("Failed to open {path}"), io::Error::last_os_error()))?;

    if handle == INVALID_HANDLE_VALUE {
        Err(DriveCkError::io(
            format!("Failed to open {path}"),
            io::Error::last_os_error(),
        ))
    } else {
        Ok(handle)
    }
}

fn query_capacity(handle: HANDLE) -> Option<u64> {
    let mut output = GET_LENGTH_INFORMATION::default();
    let mut bytes_returned = 0u32;
    let success = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_GET_LENGTH_INFO,
            None,
            0,
            Some((&mut output as *mut GET_LENGTH_INFORMATION).cast()),
            size_of::<GET_LENGTH_INFORMATION>() as u32,
            Some(&mut bytes_returned),
            None,
        )
    }
    .is_ok();
    success.then_some(output.Length as u64)
}

fn query_descriptor(handle: HANDLE) -> Option<DescriptorInfo> {
    let mut query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: STORAGE_QUERY_TYPE(PropertyStandardQuery.0),
        AdditionalParameters: [0],
    };
    let mut buffer = [0u8; 1024];
    let mut bytes_returned = 0u32;
    let success = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some((&mut query as *mut STORAGE_PROPERTY_QUERY).cast()),
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(buffer.as_mut_ptr().cast()),
            buffer.len() as u32,
            Some(&mut bytes_returned),
            None,
        )
    }
    .is_ok();
    if !success || bytes_returned < size_of::<STORAGE_DEVICE_DESCRIPTOR>() as u32 {
        return None;
    }

    let descriptor = unsafe { &*(buffer.as_ptr().cast::<STORAGE_DEVICE_DESCRIPTOR>()) };
    let vendor = read_ansi_field(&buffer, descriptor.VendorIdOffset);
    let model = read_ansi_field(&buffer, descriptor.ProductIdOffset);
    let bus_type = descriptor.BusType;
    let is_usb = bus_type == BUS_TYPE_USB;
    let transport = match bus_type {
        BUS_TYPE_USB => "usb",
        BUS_TYPE_ATA | BUS_TYPE_SATA => "ata",
        BUS_TYPE_SCSI => "scsi",
        BUS_TYPE_NVME => "nvme",
        BUS_TYPE_SD => "sd",
        _ => "block",
    };

    Some(DescriptorInfo {
        vendor,
        model,
        transport: transport.to_string(),
        is_usb,
        is_removable: descriptor.RemovableMedia,
    })
}

fn read_ansi_field(buffer: &[u8], offset: u32) -> String {
    if offset == 0 {
        return String::new();
    }
    let start = offset as usize;
    if start >= buffer.len() {
        return String::new();
    }
    let end = buffer[start..]
        .iter()
        .position(|byte| *byte == 0)
        .map(|len| start + len)
        .unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[start..end])
        .trim()
        .to_string()
}

fn parse_physical_drive_index(path: &str) -> Option<u32> {
    path.strip_prefix(r"\\.\PhysicalDrive")?.parse().ok()
}

fn wide(value: &str) -> Vec<u16> {
    PathBuf::from(value)
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
