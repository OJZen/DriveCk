# DriveCk

DriveCk validates removable and USB storage devices by sampling the device,
checking read and write integrity, and generating a human-readable report.

This repository includes the shared validation engine together with multiple
frontends:

- a command-line interface
- a native macOS CLI
- a native macOS SwiftUI + AppKit application
- a Linux GTK application
- a native Win32 application

## Highlights

- 18 x 32 validation map with 576 sampled regions
- randomized non-repeating sample order
- read / write / read-back / restore validation cycle
- validated-drive-size and highest-valid-region summary fields
- timing statistics and text report generation
- Linux removable and USB device discovery with mounted-device safety checks
- macOS-native disk discovery, CLI, and GUI layered on Rust FFI

## Repository Layout

| Crate | Purpose |
| --- | --- |
| `crates/driveck-core` | Shared target discovery, validation engine, timing math, template expansion, and text report generation |
| `crates/driveck-ffi` | C ABI bridge used by native frontends |
| `crates/driveck-cli` | Command-line frontend with list / validate / save-report flow |
| `crates/driveck-gtk` | Linux GTK frontend |
| `crates/driveck-win32` | Windows Win32 frontend |
| `script/` | Local helper scripts for Rust/macOS builds, runs, and verification |

## macOS Layout

| Path | Purpose |
| --- | --- |
| `docs/macos-requirements.md` | Detailed macOS product requirements |
| `docs/macos-design.md` | Detailed macOS architecture and UX design |
| `macos/DriveCkMacShared` | Shared Swift models, disk discovery, FFI bridge, validation orchestration, report export |
| `macos/DriveCkMacCLI` | Native Swift macOS CLI target |
| `macos/DriveCkMacApp` | SwiftUI + AppKit macOS app target |
| `macos/DriveCkMac.xcodeproj` | Xcode project for the macOS CLI and GUI |
| `macos/Scripts/build-rust-ffi.sh` | Xcode build phase script that builds `driveck-ffi` as a static library |

## Build

### Common Commands

```bash
cargo build
cargo build --workspace
cargo test --workspace
./script/build.sh cli release
./script/build.sh gtk release
./script/build.sh macos-app release
```

At the workspace root, `cargo build` covers the shared core and CLI by default.
Use `cargo build --workspace` when you intentionally want to build every
frontend.

### Helper Scripts

```bash
./script/build.sh workspace
./script/build.sh cli release
./script/build.sh gtk release
./script/build.sh win32 release
./script/build.sh macos-cli debug
./script/build.sh macos-app release
./script/package_release.sh cli
./script/package_release.sh gtk
./script/package_release.sh macos-app --snapshot
./script/build_and_run.sh run
./script/verify_all.sh
```

- `build.sh` is the primary build entrypoint. It accepts `workspace`, `core`, `ffi`, `cli`, `gtk`, `win32`, `macos-cli`, and `macos-app`, and normalizes `debug` / `release` across Cargo and Xcode builds.
- `package_release.sh` builds a release bundle and writes a concise archive named `DriveCk[-cli]-<platform>-<arch>-v<version>[+<shortsha>[.dirty]]`.
- `build_rust.sh`, `build_macos_cli.sh`, and `build_macos_app.sh` remain as compatibility wrappers around `build.sh`.
- `build_and_run.sh` rebuilds and launches the macOS app, but refuses to replace
  a running instance automatically so an in-progress validation cannot be
  interrupted mid-restore.
- `verify_all.sh` runs `cargo test --workspace`, both macOS builds, and the
  Windows cross-check when `x86_64-pc-windows-gnu` is installed.

### macOS Requirements

- Xcode with the macOS SDK and Swift 6 support
- a Rust toolchain with `cargo` on `PATH`

The macOS targets are built from `macos/DriveCkMac.xcodeproj`. During the
build, Xcode runs `macos/Scripts/build-rust-ffi.sh`, which compiles
`crates/driveck-ffi` into a static library and links it into the native app and
CLI.

### Windows Requirements

- Rust with the `x86_64-pc-windows-msvc` toolchain
- Visual Studio 2022 or Build Tools with the **Desktop development with C++** workload
- a shell that has the MSVC linker and Windows SDK on `PATH`, such as Developer PowerShell for VS 2022 or the x64 Native Tools Command Prompt

The Win32 frontend is a native Rust application that talks to the shared engine
through `driveck-ffi`, so Windows builds should use the MSVC toolchain rather
than the GNU target.

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

When the GTK app starts a validation run on Linux, it now requests administrator
access through a GUI `pkexec` / polkit prompt before opening the raw block
device, unless the app is already running as root.

Linux desktop icon assets now live under `icon/linux/`, with a matching
`icon/linux/com.github.driveck.desktop` entry and `icon/linux/hicolor/` theme
layout for the `com.github.driveck` application ID. The GTK binary also embeds
the same icon set through GResource, so it can resolve the app icon even before
the desktop entry and theme assets are installed system-wide.

Win32 on Windows (run these from Developer PowerShell for VS 2022 or another
shell initialized with the MSVC build environment):

```powershell
cargo run -p driveck-win32
cargo build --release -p driveck-win32
```

The dashboard supports:

- refreshing discovered removable and USB whole-disk targets
- checking device size, transport, and mounted status before a run starts
- watching a live 18 x 32 validation map, progress bar, metrics, and summary while validation is running
- previewing and saving the shared Rust text report after the run finishes

Mounted targets stay blocked until every partition or volume on the physical
disk has been unmounted.

### Release Packaging

Use `./script/package_release.sh <target>` on the matching host platform to
build and stage a release archive:

```bash
./script/package_release.sh cli
./script/package_release.sh gtk
./script/package_release.sh macos-app
```

Representative archive names:

- `DriveCk-cli-linux-x86_64-v0.1.0.tar.gz`
- `DriveCk-linux-x86_64-v0.1.0.tar.gz`
- `DriveCk-cli-macos-arm64-v0.1.0.zip`
- `DriveCk-macos-arm64-v0.1.0.zip`
- `DriveCk-windows-x86_64-v0.1.0.zip`

Archive names stay version-only by default. Pass `--snapshot` to append
`+<shortsha>` from a clean checkout or `+<shortsha>.dirty` from a dirty one.

The release packaging flow standardizes the staged product names too:

- CLI archives expose `driveck`
- GTK archives expose `driveck` plus `icon/linux/`
- Win32 archives expose `DriveCk.exe`
- macOS app archives expose `DriveCk.app` plus the `driveck` helper next to the app bundle

This repository still does not include an installer, MSI/MSIX manifest, or
code-signing pipeline.

macOS CLI from Xcode:

```bash
xcodebuild -project macos/DriveCkMac.xcodeproj -scheme DriveCkMacCLI -configuration Debug build
./macos/Build/Debug/driveck --list
./macos/Build/Debug/driveck --yes disk2
./macos/Build/Debug/driveck --yes --output report.txt /dev/rdisk2
```

macOS app from Xcode:

```bash
xcodebuild -project macos/DriveCkMac.xcodeproj -scheme DriveCkMacApp -configuration Debug build
open ./macos/Build/Debug/DriveCk.app
```

Cross-check the Windows frontend from Linux:

```bash
cargo check --target x86_64-pc-windows-gnu -p driveck-core -p driveck-win32
```

## Platform Notes

- The macOS frontends are implemented in Swift and the Win32 frontend is implemented in Rust, but both native frontends call the shared Rust engine through `driveck-ffi`.
- On macOS, device discovery happens in the native Swift layer and validation is executed through FFI using the discovered `TargetInfo`.
- The shared Rust validation engine now rejects mounted targets before opening the device, including requests that arrive through `driveck-ffi`.
- On macOS, validation should only be run against unmounted removable whole-disk targets. Raw disk access may require elevated privileges, and DriveCk now opens the raw device with an exclusive lock so other tools should be closed first.
- On Windows, mounted-volume detection maps volume mount points back to their physical drives before validation is allowed, and the Win32 dashboard now lists devices, validates targets, and renders reports through `driveck-ffi`.
- The GTK frontend is compiled only on Linux targets.
- On Linux, the GTK frontend uses `pkexec --disable-internal-agent` for GUI privilege elevation before validation, so a running polkit authentication agent is required when the app is not already elevated.
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
