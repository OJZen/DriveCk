# DriveCk

DriveCk validates removable and USB storage devices by sampling the device,
checking read and write integrity, and generating a human-readable report.

This repository includes the shared validation engine together with multiple
frontends:

- a command-line interface
- a Linux GTK application
- a native Win32 application

## Highlights

- 24 x 24 validation map with 576 sampled regions
- randomized non-repeating sample order
- read / write / read-back / restore validation cycle
- validated-drive-size and highest-valid-region summary fields
- timing statistics and text report generation
- Linux removable and USB device discovery with mounted-device safety checks

## Repository Layout

| Crate | Purpose |
| --- | --- |
| `crates/driveck-core` | Shared target discovery, validation engine, timing math, template expansion, and text report generation |
| `crates/driveck-cli` | Command-line frontend with list / validate / save-report flow |
| `crates/driveck-gtk` | Linux GTK frontend |
| `crates/driveck-win32` | Windows Win32 frontend |

## Build

### Common Commands

```bash
cargo build
cargo build --workspace
cargo test --workspace
cargo build --release -p driveck-cli
```

At the workspace root, `cargo build` covers the shared core and CLI by default.
Use `cargo build --workspace` when you intentionally want to build every
frontend.

### Run

CLI:

```bash
cargo run -p driveck-cli -- --list
cargo run -p driveck-cli -- --yes /dev/sdb
cargo run -p driveck-cli -- --yes --output report.txt /dev/sdb
```

GTK on Linux:

```bash
cargo run -p driveck-gtk
```

Win32 on Windows:

```powershell
cargo run -p driveck-win32
```

Cross-check the Windows frontend from Linux:

```bash
cargo check --target x86_64-pc-windows-gnu -p driveck-core -p driveck-win32
```

## Platform Notes

- The GTK frontend is compiled only on Linux targets.
- The Win32 frontend is compiled only on Windows targets.
- Validation targets must be whole block devices. Regular files and partitions are rejected.

## Build Size Notes

The largest artifacts in `target/` usually come from:

- the GTK dependency stack (`gtk4`, `glib`, `gio`, `pango`, `cairo`)
- debug builds that keep full debug information and incremental state
- cross-target builds such as `target/x86_64-pc-windows-gnu/`

If you only need the CLI, prefer:

```bash
cargo build --release -p driveck-cli
```

## References

- [GRC ValiDrive](https://www.grc.com/validrive.htm)
