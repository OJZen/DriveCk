#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

TARGET="${1:-workspace}"
PROFILE_INPUT="${2:-debug}"

usage() {
  echo "usage: $0 [workspace|core|ffi|cli|gtk|win32|macos-cli|macos-app] [debug|release]" >&2
}

PROFILE="$(normalize_build_profile "$PROFILE_INPUT" || true)"
if [ -z "$PROFILE" ]; then
  usage
  exit 2
fi

run_cargo_build() {
  if [ "$PROFILE" = "release" ]; then
    cargo_cmd build "$@" --release
  else
    cargo_cmd build "$@"
  fi
}

run_xcode_build() {
  local configuration
  configuration="$(xcode_configuration_for_profile "$PROFILE")"
  xcodebuild_cmd -scheme "$1" -configuration "$configuration" build
}

case "$TARGET" in
  workspace)
    run_cargo_build --workspace
    ;;
  core)
    run_cargo_build -p driveck-core
    ;;
  ffi)
    run_cargo_build -p driveck-ffi
    ;;
  cli)
    run_cargo_build -p driveck-cli
    ;;
  gtk)
    run_cargo_build -p driveck-gtk
    ;;
  win32)
    run_cargo_build -p driveck-win32
    ;;
  macos-cli)
    run_xcode_build "$MACOS_CLI_TARGET"
    ;;
  macos-app)
    run_xcode_build "$MACOS_APP_TARGET"
    ;;
  *)
    usage
    exit 2
    ;;
esac
