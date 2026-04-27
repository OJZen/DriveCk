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
| `resources/` | Shared screenshots and application assets |
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

On Windows PowerShell, prefer the wrapper:

```powershell
.\script\package_release.ps1 win32
.\script\package_release.ps1 cli
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
./script/package_release.sh win32
./script/package_release.sh macos-app
./script/package_release.sh macos-app --snapshot
```

Archive naming:

- CLI packages: `DriveCk-cli-<platform>-<arch>-v<version>`
- GUI packages: `DriveCk-gui-<platform>-<arch>-v<version>`

Examples:

- `DriveCk-cli-linux-x86_64-v0.1.0.tar.gz`
- `DriveCk-gui-linux-x86_64-v0.1.0.tar.gz`
- `DriveCk-cli-windows-x86_64-v0.1.0.zip`
- `DriveCk-gui-windows-x86_64-v0.1.0.zip`
- `DriveCk-cli-macos-arm64-v0.1.0.zip`
- `DriveCk-gui-macos-arm64-v0.1.0.zip`

Pass `--snapshot` to append `+<shortsha>` from a clean checkout or
`+<shortsha>.dirty` from a dirty checkout.

Packaging also standardizes staged product names:

- CLI packages expose `driveck` on Unix-like hosts and `driveck.exe` on Windows
- Linux GUI packages expose `driveck` plus `resources/icon/linux/`
- Windows GUI packages expose `DriveCk.exe`, which can launch the GUI or run the embedded CLI mode
- macOS GUI packages expose `DriveCk.app` plus the `driveck` helper

Release archives intentionally omit `README.md` and only stage the runtime
artifacts needed by that target.

## Linux installer

The repository now includes a Linux installer script:

```bash
./script/install_linux.sh --user target/release/DriveCk-gui-linux-x86_64-v0.1.0.tar.gz
./script/install_linux.sh --system target/release/DriveCk-cli-linux-x86_64-v0.1.0.tar.gz
```

Install locations follow standard local paths instead of writing arbitrary files:

| Mode | Payload root | Executable entry | Desktop / icon data |
| --- | --- | --- | --- |
| `--user` | `~/.local/lib/driveck` | `~/.local/bin/driveck` for CLI | `~/.local/share/applications`, `~/.local/share/icons` |
| `--system` | `/usr/local/lib/driveck` | `/usr/local/bin/driveck` for CLI | `/usr/local/share/applications`, `/usr/local/share/icons` |

Behavior:

- GUI installs keep the application binary under the managed payload root and
  install a `.desktop` launcher plus hicolor icons.
- GUI installs also expose a `driveck-gui` symlink so the same binary can be
  used in GUI mode or CLI mode.
- CLI installs keep the payload versioned under the managed root and expose a
  `driveck` symlink in the standard bin directory.
- Existing unmanaged files are never overwritten.

The matching uninstaller is:

```bash
./script/uninstall_linux.sh --user --all
./script/uninstall_linux.sh --system --gui
```

It removes only DriveCk-managed GUI/CLI installs from the selected scope and
leaves unrelated files alone.

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

The GTK binary now behaves like this on Linux:

- no arguments: start the GUI
- CLI arguments such as `--list` or `--yes /dev/sdb`: run in CLI mode
- `--validate-helper`: reserved for the internal privileged helper flow

### Win32

Run these from a Windows shell initialized with the MSVC environment:

```powershell
cargo run -p driveck-win32
cargo build --release -p driveck-win32
```

The Win32 executable also supports the embedded CLI mode when launched with
arguments:

```powershell
cargo run -p driveck-win32 -- --list
.\target\release\driveck-win32.exe --yes \\.\PhysicalDrive2
.\target\release\driveck-win32.exe --gui
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
