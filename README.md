# DriveCk

DriveCk validates removable and USB storage devices, checks read/write
integrity on sampled regions, and generates a human-readable report.

## Platforms

- Linux: GTK GUI and CLI
- macOS: native app and native CLI
- Windows: native Win32 app

## Screenshots

Current screenshots are from the Linux GTK frontend. macOS and Windows
screenshots can be added alongside them later.

![Linux GTK target selection](resources/imgs/linux-gtk-first.png)

![Linux GTK validation result](resources/imgs/linux-gtk-good.png)

## Features

- validates removable and USB whole-disk devices
- live 18 x 32 validation grid during a run
- human-readable report with verdict and usable-size summary
- Linux GTK app, Linux CLI, native macOS app/CLI, and native Win32 app

## Before you run it

- Validate only whole devices, not partitions or image files.
- Sampled regions are temporarily overwritten and then restored.
- Unmount the target before validation.
- Raw-device access may require administrator privileges.

## Quick start

### Linux GTK

Run from source:

```bash
cargo run -p driveck-gtk
```

If you use the packaged Linux GTK release, extract the archive and run:

```bash
./driveck
```

When the GTK app needs elevated access on Linux, it requests it through a GUI
authentication prompt.

### CLI

Run from source:

```bash
cargo run -p driveck-cli -- --list
cargo run -p driveck-cli -- --yes /dev/sdb
cargo run -p driveck-cli -- --yes --output report.txt /dev/sdb
```

CLI release packages also extract to a short executable name:

```bash
./driveck --list
./driveck --yes /dev/sdb
```

### macOS and Windows

- macOS uses native frontends: a SwiftUI + AppKit app and a native CLI.
- Windows uses a native Win32 frontend.
- Additional screenshots for those frontends can be added to the same
  `resources/imgs/` area later.

## Release package names

- GUI packages: `DriveCk-<platform>-<arch>-v<version>`
- CLI packages: `DriveCk-cli-<platform>-<arch>-v<version>`

Examples:

- `DriveCk-linux-x86_64-v0.1.0.tar.gz`
- `DriveCk-cli-linux-x86_64-v0.1.0.tar.gz`

## Developer docs

Build, packaging, and platform-specific developer details now live here:

- [Developer guide](docs/development.md)
- [macOS requirements](docs/macos-requirements.md)
- [macOS design notes](docs/macos-design.md)

## Reference

- [GRC ValiDrive](https://www.grc.com/validrive.htm)
