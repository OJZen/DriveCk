#!/usr/bin/env bash

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROJECT_PATH="$ROOT_DIR/macos/DriveCkMac.xcodeproj"
MACOS_APP_NAME="DriveCkMacApp"
MACOS_BUNDLE_ID="io.github.ojzen.driveck.macos.app"

cargo_cmd() {
  if command -v cargo >/dev/null 2>&1; then
    cargo "$@"
    return
  fi
  if [ -x "$HOME/.cargo/bin/cargo" ]; then
    "$HOME/.cargo/bin/cargo" "$@"
    return
  fi

  echo "cargo was not found. Install Rust or add cargo to PATH." >&2
  exit 127
}

xcodebuild_cmd() {
  xcodebuild -project "$PROJECT_PATH" "$@"
}

app_is_running() {
  pgrep -x "$MACOS_APP_NAME" >/dev/null 2>&1
}
