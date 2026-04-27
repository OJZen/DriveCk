# DriveCk developer guide

This document collects the build, packaging, and platform-specific details that
are intentionally kept out of the main README.

## Workspace layout

| Path | Purpose |
| --- | --- |
| `crates/driveck-core` | Shared target discovery, validation engine, report generation |
| `crates/driveck-ffi` | C ABI bridge used by native frontends |
| `crates/driveck-cli` | Rust CLI frontend |
| `crates/driveck-gtk` | Linux GTK frontend |
| `crates/driveck-win32` | Native Win32 frontend |
| `macos/DriveCkMacCLI` | Native macOS CLI target |
| `macos/DriveCkMacApp` | Native macOS app target |
| `macos/DriveCkMac.xcodeproj` | Xcode project for the macOS native frontends |
| `script/` | Local build, run, verification, and packaging helpers |

## Unified build entrypoint

The primary local build command is:

```bash
./script/build.sh <target> <debug|release>
```

Supported targets:

- `workspace`
- `core`
- `ffi`
- `cli`
- `gtk`
- `win32`
- `macos-cli`
- `macos-app`

Examples:

```bash
./script/build.sh workspace
./script/build.sh cli release
./script/build.sh gtk release
./script/build.sh win32 release
./script/build.sh macos-cli debug
./script/build.sh macos-app release
```

Compatibility wrappers still exist:

- `script/build_rust.sh`
- `script/build_macos_cli.sh`
- `script/build_macos_app.sh`

## Verification

Use the shared verification script:

```bash
./script/verify_all.sh
```

It runs:

- `cargo test --workspace`
- the macOS CLI build
- the macOS app build
- the Windows GNU cross-check when `x86_64-pc-windows-gnu` is installed

## Packaging

Use:

```bash
./script/package_release.sh <target>
```

Supported packaging targets:

- `cli`
- `gtk`
- `win32`
- `macos-cli`
- `macos-app`

Examples:

```bash
./script/package_release.sh cli
./script/package_release.sh gtk
./script/package_release.sh macos-app
./script/package_release.sh macos-app --snapshot
```

Archive naming:

- GUI packages: `DriveCk-<platform>-<arch>-v<version>`
- CLI packages: `DriveCk-cli-<platform>-<arch>-v<version>`

Examples:

- `DriveCk-linux-x86_64-v0.1.0.tar.gz`
- `DriveCk-cli-linux-x86_64-v0.1.0.tar.gz`
- `DriveCk-macos-arm64-v0.1.0.zip`
- `DriveCk-windows-x86_64-v0.1.0.zip`

Pass `--snapshot` to append `+<shortsha>` from a clean checkout or
`+<shortsha>.dirty` from a dirty checkout.

Packaging also standardizes staged product names:

- CLI packages expose `driveck`
- GTK packages expose `driveck` plus `icon/linux/`
- Win32 packages expose `DriveCk.exe`
- macOS app packages expose `DriveCk.app` plus the `driveck` helper

## Running locally

### Rust CLI

```bash
cargo run -p driveck-cli -- --list
cargo run -p driveck-cli -- --yes /dev/sdb
cargo run -p driveck-cli -- --yes --output report.txt /dev/sdb
```

### Linux GTK

```bash
cargo run -p driveck-gtk
```

### Win32

Run these from a Windows shell initialized with the MSVC environment:

```powershell
cargo run -p driveck-win32
cargo build --release -p driveck-win32
```

### macOS CLI

```bash
xcodebuild -project macos/DriveCkMac.xcodeproj -scheme DriveCkMacCLI -configuration Debug build
./macos/Build/Debug/driveck --list
./macos/Build/Debug/driveck --yes disk2
./macos/Build/Debug/driveck --yes --output report.txt /dev/rdisk2
```

### macOS app

```bash
xcodebuild -project macos/DriveCkMac.xcodeproj -scheme DriveCkMacApp -configuration Debug build
open ./macos/Build/Debug/DriveCk.app
```

## Platform notes

- Validation targets must be whole block devices. Regular files and partitions
  are rejected.
- The shared Rust validation engine rejects mounted targets before opening the
  device.
- On Linux, the GTK frontend requests elevated access through
  `pkexec --disable-internal-agent` when it is not already running as root.
- On macOS, validation runs through `driveck-ffi`, and the app expects the
  native CLI helper (`driveck`) beside the app bundle when packaged.
- On Windows, the Win32 frontend is a native Rust app layered on `driveck-ffi`.
- Windows release builds are intended to use the `x86_64-pc-windows-msvc`
  toolchain.
- Linux can run a Windows GNU cross-check with:

  ```bash
  cargo check --target x86_64-pc-windows-gnu -p driveck-core -p driveck-win32
  ```

  That cross-check does not replace a native Windows/MSVC release build.

## Related docs

- [macOS requirements](macos-requirements.md)
- [macOS design notes](macos-design.md)
