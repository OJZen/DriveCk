use std::{fs::OpenOptions, path::PathBuf};

use driveck_core::discover_target;
use tempfile::tempdir;

#[test]
fn rejects_regular_files() {
    let directory = tempdir().expect("tempdir");
    let file_path: PathBuf = directory.path().join("sample.bin");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(true)
        .open(&file_path)
        .expect("regular file");
    file.set_len(3 * 1024 * 1024).expect("resize file");

    let error = discover_target(&file_path).expect_err("regular files must be rejected");
    assert!(
        error.message.contains("block device") || error.message.contains("physical drive path"),
        "unexpected error: {}",
        error.message
    );
}
