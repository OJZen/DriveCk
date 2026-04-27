#!/usr/bin/env bash

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROJECT_PATH="$ROOT_DIR/macos/DriveCkMac.xcodeproj"
MACOS_APP_TARGET="DriveCkMacApp"
MACOS_CLI_TARGET="DriveCkMacCLI"
MACOS_APP_NAME="DriveCk"
MACOS_CLI_NAME="driveck"
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

normalize_build_profile() {
  case "${1:-debug}" in
    debug|Debug)
      printf '%s\n' "debug"
      ;;
    release|Release)
      printf '%s\n' "release"
      ;;
    *)
      return 1
      ;;
  esac
}

xcode_configuration_for_profile() {
  case "${1:-debug}" in
    debug)
      printf '%s\n' "Debug"
      ;;
    release)
      printf '%s\n' "Release"
      ;;
    *)
      return 1
      ;;
  esac
}

host_os_id() {
  case "$(uname -s)" in
    Linux)
      printf '%s\n' "linux"
      ;;
    Darwin)
      printf '%s\n' "macos"
      ;;
    MINGW*|MSYS*|CYGWIN*)
      printf '%s\n' "windows"
      ;;
    *)
      return 1
      ;;
  esac
}

host_arch_id() {
  case "$(uname -m)" in
    x86_64|amd64)
      printf '%s\n' "x86_64"
      ;;
    aarch64|arm64)
      printf '%s\n' "arm64"
      ;;
    *)
      uname -m
      ;;
  esac
}

workspace_version() {
  sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT_DIR/Cargo.toml" | head -n 1
}

git_worktree_dirty() {
  local status

  if ! command -v git >/dev/null 2>&1; then
    return 1
  fi

  status="$(git -C "$ROOT_DIR" status --porcelain --untracked-files=normal 2>/dev/null || true)"
  [ -n "$status" ]
}

git_snapshot_suffix() {
  local mode="${1:-none}"
  local short_sha

  if ! command -v git >/dev/null 2>&1; then
    printf '%s\n' ""
    return
  fi

  short_sha="$(git -C "$ROOT_DIR" rev-parse --short HEAD 2>/dev/null || true)"
  if [ -z "$short_sha" ]; then
    printf '%s\n' ""
    return
  fi

  if [ "$mode" != "snapshot" ]; then
    printf '%s\n' ""
  elif git_worktree_dirty; then
    printf '+%s.dirty\n' "$short_sha"
  else
    printf '+%s\n' "$short_sha"
  fi
}
