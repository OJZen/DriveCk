use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io,
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        fs::{FileExt, OpenOptionsExt},
        io::{FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_MORE_DATA, ERROR_NO_MORE_FILES,
            ERROR_NOT_READY, HANDLE, INVALID_HANDLE_VALUE,
        },
        Storage::FileSystem::{
            BusTypeAta as BUS_TYPE_ATA, BusTypeNvme as BUS_TYPE_NVME, BusTypeSata as BUS_TYPE_SATA,
            BusTypeScsi as BUS_TYPE_SCSI, BusTypeSd as BUS_TYPE_SD, BusTypeUsb as BUS_TYPE_USB,
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_NO_BUFFERING, FILE_FLAG_WRITE_THROUGH,
            FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose,
            IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, OPEN_EXISTING,
        },
        System::{
            IO::DeviceIoControl,
            Ioctl::{
                DISK_EXTENT, DISK_GEOMETRY_EX, FSCTL_DISMOUNT_VOLUME, FSCTL_IS_VOLUME_MOUNTED,
                FSCTL_LOCK_VOLUME, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_STORAGE_QUERY_PROPERTY,
                PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY,
                STORAGE_QUERY_TYPE, StorageDeviceProperty, VOLUME_DISK_EXTENTS,
            },
        },
    },
    core::PCWSTR,
};

use crate::{DriveCkError, TargetInfo, TargetKind};

static VOLUME_LOCKS: OnceLock<Mutex<HashMap<u32, Vec<OwnedHandle>>>> = OnceLock::new();

fn volume_locks() -> &'static Mutex<HashMap<u32, Vec<OwnedHandle>>> {
    VOLUME_LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn volume_lock_exists(index: u32) -> bool {
    match volume_locks().lock() {
        Ok(active) => active.contains_key(&index),
        Err(poisoned) => poisoned.into_inner().contains_key(&index),
    }
}

fn store_volume_locks(index: u32, handles: Vec<OwnedHandle>) {
    match volume_locks().lock() {
        Ok(mut active) => {
            active.insert(index, handles);
        }
        Err(poisoned) => {
            poisoned.into_inner().insert(index, handles);
        }
    }
}

fn release_volume_locks(index: u32) {
    match volume_locks().lock() {
        Ok(mut active) => {
            active.remove(&index);
        }
        Err(poisoned) => {
            poisoned.into_inner().remove(&index);
        }
    }
}

pub(crate) struct OpenedTarget {
    file: File,
    direct_io_used: bool,
}

impl OpenedTarget {
    pub(crate) fn open(target: &TargetInfo) -> Result<Self, DriveCkError> {
        let mut options = OpenOptions::new();
        options.read(true).write(true);

        if target.is_block_device {
            if let Some(index) = parse_physical_drive_index(&target.path) {
                if is_physical_drive_mounted(index)? {
                    return Err(DriveCkError::new(format!(
                        "Refusing to validate {} because the disk or one of its volumes is mounted.",
                        target.path
                    )));
                }
            }
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
        if let Ok(target) = query_physical_drive(index, false) {
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
        return query_physical_drive(index, true);
    }

    Err(DriveCkError::new(format!(
        "Target {} is not a physical drive path.",
        path.display()
    )))
}

pub(crate) fn inspect_target(path: &Path) -> Result<TargetInfo, DriveCkError> {
    let path_text = path.to_string_lossy().to_string();
    if let Some(index) = parse_physical_drive_index(&path_text) {
        return query_physical_drive(index, false);
    }

    Err(DriveCkError::new(format!(
        "Target {} is not a physical drive path.",
        path.display()
    )))
}

pub(crate) fn unmount_target(path: &Path) -> Result<TargetInfo, DriveCkError> {
    let path_text = path.to_string_lossy().to_string();
    let Some(index) = parse_physical_drive_index(&path_text) else {
        return Err(DriveCkError::new(format!(
            "Target {} is not a physical drive path.",
            path.display()
        )));
    };

    if !volume_lock_exists(index) {
        let mut handles = Vec::new();
        for volume_name in mapped_mounted_volumes(index)? {
            handles.push(lock_volume(volume_name.as_str())?);
        }
        if !handles.is_empty() {
            store_volume_locks(index, handles);
        }
    }

    match query_physical_drive(index, true) {
        Ok(target) => Ok(target),
        Err(error) => {
            release_volume_locks(index);
            Err(error)
        }
    }
}

pub(crate) fn release_unmount_target(path: &Path) -> Result<(), DriveCkError> {
    let path_text = path.to_string_lossy().to_string();
    let Some(index) = parse_physical_drive_index(&path_text) else {
        return Err(DriveCkError::new(format!(
            "Target {} is not a physical drive path.",
            path.display()
        )));
    };

    release_volume_locks(index);
    Ok(())
}

fn query_physical_drive(index: u32, reject_mounted: bool) -> Result<TargetInfo, DriveCkError> {
    let path = format!(r"\\.\PhysicalDrive{index}");
    let handle = open_metadata_handle(&path)?;
    let capacity = query_capacity(handle)
        .ok_or_else(|| DriveCkError::new(format!("Failed to query capacity for {path}.")))?;
    let descriptor = query_descriptor(handle).unwrap_or_default();
    unsafe {
        let _ = CloseHandle(handle);
    }
    let mounted = is_physical_drive_mounted(index)?;
    if reject_mounted && mounted {
        return Err(DriveCkError::new(format!(
            "Refusing to validate {} because the disk or one of its volumes is mounted.",
            path
        )));
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
        is_mounted: mounted,
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
            0,
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
    let mut buffer = [0u8; 256];
    let mut bytes_returned = 0u32;
    let success = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
            None,
            0,
            Some(buffer.as_mut_ptr().cast()),
            buffer.len() as u32,
            Some(&mut bytes_returned),
            None,
        )
    }
    .is_ok();
    if !success || bytes_returned < size_of::<DISK_GEOMETRY_EX>() as u32 {
        return None;
    }

    let geometry = unsafe { &*(buffer.as_ptr().cast::<DISK_GEOMETRY_EX>()) };
    Some(geometry.DiskSize as u64)
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

fn mapped_mounted_volumes(index: u32) -> Result<Vec<String>, DriveCkError> {
    let mut volumes = Vec::new();
    for volume_name in list_volume_names()? {
        if volume_maps_to_disk(&volume_name, index)? && is_volume_mounted(&volume_name)? {
            volumes.push(volume_name);
        }
    }
    Ok(volumes)
}

fn is_physical_drive_mounted(index: u32) -> Result<bool, DriveCkError> {
    if volume_lock_exists(index) {
        return Ok(false);
    }
    Ok(!mapped_mounted_volumes(index)?.is_empty())
}

fn list_volume_names() -> Result<Vec<String>, DriveCkError> {
    let mut buffer = vec![0u16; 1024];
    let handle = unsafe { FindFirstVolumeW(&mut buffer) }.map_err(|_| {
        DriveCkError::io(
            "Failed to enumerate Windows volumes",
            io::Error::last_os_error(),
        )
    })?;

    let mut volumes = Vec::new();
    let result = (|| {
        loop {
            let volume_name = wide_buffer_to_string(&buffer);
            if !volume_name.is_empty() {
                volumes.push(volume_name);
            }

            buffer.fill(0);
            if unsafe { FindNextVolumeW(handle, &mut buffer) }.is_ok() {
                continue;
            }

            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES.0 as i32) {
                return Ok(volumes);
            }
            return Err(DriveCkError::io(
                "Failed to continue Windows volume enumeration",
                error,
            ));
        }
    })();

    unsafe {
        let _ = FindVolumeClose(handle);
    }
    result
}

fn volume_maps_to_disk(volume_name: &str, index: u32) -> Result<bool, DriveCkError> {
    let handle = open_volume_handle(volume_name)?;
    let result = query_volume_disk_numbers(handle).map(|disk_numbers| {
        disk_numbers
            .into_iter()
            .any(|disk_number| disk_number == index)
    });
    unsafe {
        let _ = CloseHandle(handle);
    }
    result
}

fn open_volume_handle(volume_name: &str) -> Result<HANDLE, DriveCkError> {
    open_volume_handle_with_access(volume_name, 0)
}

fn open_volume_handle_with_access(
    volume_name: &str,
    desired_access: u32,
) -> Result<HANDLE, DriveCkError> {
    let path = volume_name.trim_end_matches('\\');
    let wide = wide(path);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            desired_access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .map_err(|_| {
        DriveCkError::io(
            format!("Failed to open volume {volume_name}"),
            io::Error::last_os_error(),
        )
    })?;

    if handle == INVALID_HANDLE_VALUE {
        Err(DriveCkError::io(
            format!("Failed to open volume {volume_name}"),
            io::Error::last_os_error(),
        ))
    } else {
        Ok(handle)
    }
}

fn is_volume_mounted(volume_name: &str) -> Result<bool, DriveCkError> {
    let handle = open_volume_handle_with_access(volume_name, FILE_GENERIC_READ.0)?;
    let mut bytes_returned = 0u32;
    let mounted = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_IS_VOLUME_MOUNTED,
            None,
            0,
            None,
            0,
            Some(&mut bytes_returned),
            None,
        )
    }
    .is_ok();
    let error = if mounted {
        None
    } else {
        Some(io::Error::last_os_error())
    };
    unsafe {
        let _ = CloseHandle(handle);
    }

    if mounted {
        Ok(true)
    } else if error.as_ref().and_then(|value| value.raw_os_error())
        == Some(ERROR_NOT_READY.0 as i32)
    {
        Ok(false)
    } else {
        Err(DriveCkError::io(
            format!("Failed to query mount state for {volume_name}"),
            error.unwrap_or_else(io::Error::last_os_error),
        ))
    }
}

fn lock_volume(volume_name: &str) -> Result<OwnedHandle, DriveCkError> {
    let handle =
        open_volume_handle_with_access(volume_name, FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0)?;
    let result = (|| {
        let mut bytes_returned = 0u32;
        let locked = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_LOCK_VOLUME,
                None,
                0,
                None,
                0,
                Some(&mut bytes_returned),
                None,
            )
        }
        .is_ok();
        if !locked {
            return Err(DriveCkError::io(
                format!(
                    "Failed to lock volume {volume_name}. Close any open files or Explorer windows on the disk and try again."
                ),
                io::Error::last_os_error(),
            ));
        }

        let dismounted = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_DISMOUNT_VOLUME,
                None,
                0,
                None,
                0,
                Some(&mut bytes_returned),
                None,
            )
        }
        .is_ok();
        if !dismounted {
            return Err(DriveCkError::io(
                format!("Failed to dismount volume {volume_name}."),
                io::Error::last_os_error(),
            ));
        }

        Ok(())
    })();
    match result {
        Ok(()) => Ok(unsafe { OwnedHandle::from_raw_handle(handle.0) }),
        Err(error) => {
            unsafe {
                let _ = CloseHandle(handle);
            }
            Err(error)
        }
    }
}

fn query_volume_disk_numbers(handle: HANDLE) -> Result<Vec<u32>, DriveCkError> {
    let mut buffer_len = 1024usize;

    loop {
        let mut buffer = vec![0u8; buffer_len];
        let mut bytes_returned = 0u32;
        let success = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
                None,
                0,
                Some(buffer.as_mut_ptr().cast()),
                buffer.len() as u32,
                Some(&mut bytes_returned),
                None,
            )
        }
        .is_ok();
        if success {
            if bytes_returned < size_of::<VOLUME_DISK_EXTENTS>() as u32 {
                return Err(DriveCkError::new(
                    "Windows volume extent query returned an unexpectedly short payload.",
                ));
            }

            let extents = unsafe { &*(buffer.as_ptr().cast::<VOLUME_DISK_EXTENTS>()) };
            let count = extents.NumberOfDiskExtents as usize;
            let required_len = size_of::<VOLUME_DISK_EXTENTS>()
                + count.saturating_sub(1) * size_of::<DISK_EXTENT>();
            if (bytes_returned as usize) < required_len {
                return Err(DriveCkError::new(
                    "Windows volume extent query returned a truncated payload.",
                ));
            }

            let first_extent = std::ptr::addr_of!(extents.Extents).cast::<DISK_EXTENT>();
            let extents = unsafe { std::slice::from_raw_parts(first_extent, count) };
            return Ok(extents.iter().map(|extent| extent.DiskNumber).collect());
        }

        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(code)
                if code == ERROR_MORE_DATA.0 as i32
                    || code == ERROR_INSUFFICIENT_BUFFER.0 as i32 =>
            {
                buffer_len = buffer_len.saturating_mul(2);
            }
            _ => {
                return Err(DriveCkError::io(
                    "Failed to query Windows volume disk extents",
                    error,
                ));
            }
        }
    }
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

fn wide_buffer_to_string(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|ch| *ch == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

fn wide(value: &str) -> Vec<u16> {
    PathBuf::from(value)
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
